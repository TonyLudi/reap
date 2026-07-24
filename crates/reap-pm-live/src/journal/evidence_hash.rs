use reap_pm_core::{
    EvmAddress, PmAccountHandle, PmAccountScope, PmAssetId, PmClientOrderKey, PmConnectionId,
    PmFillId, PmInstrumentId, PmQuantity, PmVenueOrderKey, U256,
};
use sha2::{Digest, Sha256};

use super::PM_SEALED_JOURNAL_RECORD_KINDS;
use super::schema::{
    PmJournalCancelOutcomeV1, PmJournalCancelReasonV1, PmJournalCancelRejectReasonV1,
    PmJournalFillDeliveryV1, PmJournalFillFeeV1, PmJournalFillKeyV1, PmJournalFillOccurrenceV1,
    PmJournalFillRoleV1, PmJournalFillSettlementV1, PmJournalFillSourceV1, PmJournalFingerprintV1,
    PmJournalOrderProgressSourceV1, PmJournalPlaceOutcomeV1, PmJournalPlaceRejectReasonV1,
    PmJournalQuoteProfileV1, PmJournalRecordV1, PmJournalSafetyReasonV1, PmJournalSideV1,
    PmJournalSignV1, PmJournalTerminalStatusV1, derive_pm_journal_client_order_from_fingerprint,
};

const SEALED_SEGMENT_HASH_DOMAIN_V1: &[u8] = b"reap-pm-live/sealed-journal-segment/v1";

pub(super) struct PmSealedJournalSegment {
    record_count: u64,
    records_by_kind: [u64; PM_SEALED_JOURNAL_RECORD_KINDS],
    hasher: Sha256,
    normalizer: Normalizer,
}

impl PmSealedJournalSegment {
    pub(super) fn new(account: PmAccountHandle, fingerprint: PmJournalFingerprintV1) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(SEALED_SEGMENT_HASH_DOMAIN_V1);
        hasher.update(fingerprint.bytes());
        Self {
            record_count: 0,
            records_by_kind: [0; PM_SEALED_JOURNAL_RECORD_KINDS],
            hasher,
            normalizer: Normalizer::new(account, fingerprint),
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new(self.normalizer.account, self.normalizer.fingerprint);
    }

    pub(super) fn observe(&mut self, sequence: u64, kind_index: usize, record: &PmJournalRecordV1) {
        self.record_count = self.record_count.saturating_add(1);
        self.records_by_kind[kind_index] = self.records_by_kind[kind_index].saturating_add(1);
        self.hasher.update([kind_index as u8]);
        let normalized_sequence = relative(
            &mut self.normalizer.sequence_base,
            sequence,
            &mut self.normalizer.valid,
        );
        self.normalizer.valid &= normalized_sequence == self.record_count;
        self.hasher.update(normalized_sequence.to_be_bytes());
        self.normalizer.hash_record(&mut self.hasher, record);
    }

    pub(super) const fn record_count(&self) -> u64 {
        self.record_count
    }

    pub(super) const fn records_by_kind(&self) -> [u64; PM_SEALED_JOURNAL_RECORD_KINDS] {
        self.records_by_kind
    }

    pub(super) const fn valid(&self) -> bool {
        self.normalizer.valid
    }

    pub(super) fn hash(&self) -> [u8; 32] {
        self.hasher.clone().finalize().into()
    }
}

struct Normalizer {
    account: PmAccountHandle,
    fingerprint: PmJournalFingerprintV1,
    valid: bool,
    sequence_base: Option<u64>,
    intent_base: Option<u64>,
    quote_ordinal: u64,
    current_client: Option<PmClientOrderKey>,
    venue_base: Option<u64>,
    fill_base: Option<u64>,
    quote_bases: [Option<u64>; 8],
    occurrence_bases: [Option<u64>; 4],
    watermark_ordinal: u64,
    previous_watermark: Option<PmJournalFingerprintV1>,
}

impl Normalizer {
    const fn new(account: PmAccountHandle, fingerprint: PmJournalFingerprintV1) -> Self {
        Self {
            account,
            fingerprint,
            valid: true,
            sequence_base: None,
            intent_base: None,
            quote_ordinal: 0,
            current_client: None,
            venue_base: None,
            fill_base: None,
            quote_bases: [None; 8],
            occurrence_bases: [None; 4],
            watermark_ordinal: 0,
            previous_watermark: None,
        }
    }

    fn hash_record(&mut self, hasher: &mut Sha256, record: &PmJournalRecordV1) {
        match record {
            PmJournalRecordV1::Header(header) => {
                hasher.update(header.scope().fingerprint().bytes());
            }
            PmJournalRecordV1::QuoteIntent(intent) => {
                self.quote_ordinal = self.quote_ordinal.saturating_add(1);
                let normalized_intent =
                    relative(&mut self.intent_base, intent.intent_id, &mut self.valid);
                self.valid &= normalized_intent == self.quote_ordinal;
                self.valid &= derive_pm_journal_client_order_from_fingerprint(
                    self.account,
                    self.fingerprint,
                    intent.intent_id,
                )
                .is_ok_and(|expected| expected == intent.client_order);
                self.current_client = Some(intent.client_order);
                hasher.update(self.quote_ordinal.to_be_bytes());
                self.hash_client(hasher, intent.client_order);
                hash_instrument(hasher, intent.instrument);
                hash_side(hasher, intent.side);
                hasher.update(intent.price_units.to_be_bytes());
                hash_quantity(hasher, intent.quantity);
                hash_u256(hasher, intent.reserved_collateral);
                hash_u256(hasher, intent.reserved_outcome);
                hash_profile(hasher, intent.profile);
                for (base, value) in self.quote_bases.iter_mut().zip([
                    intent.metadata_revision,
                    intent.book_revision,
                    intent.model_revision,
                    intent.book_readiness_revision,
                    intent.private_readiness_revision,
                    intent.expires_at_monotonic_ns,
                    intent.salt.value(),
                    intent.timestamp_ms,
                ]) {
                    hasher.update(relative(base, value, &mut self.valid).to_be_bytes());
                }
                hash_address(hasher, intent.maker);
                hash_address(hasher, intent.signer);
                hash_u256(hasher, intent.maker_amount);
                hash_u256(hasher, intent.taker_amount);
            }
            PmJournalRecordV1::PlaceResult(result) => {
                self.hash_client(hasher, result.client_order);
                hash_place_outcome(hasher, result.outcome);
                hash_place_reject_reason(hasher, result.reject_reason);
                self.hash_optional_venue(hasher, result.venue_order);
                let fills = result.immediate_fills.iter().collect::<ArrayVec64>();
                hasher.update([fills.len]);
                for key in fills
                    .values
                    .into_iter()
                    .take(usize::from(fills.len))
                    .flatten()
                {
                    self.hash_fill_key(hasher, key);
                }
            }
            PmJournalRecordV1::CancelIntent(intent) => {
                self.hash_client(hasher, intent.client_order);
                self.hash_venue(hasher, intent.venue_order);
                hash_cancel_reason(hasher, intent.reason);
            }
            PmJournalRecordV1::CancelResult(result) => {
                self.hash_client(hasher, result.client_order);
                self.hash_venue(hasher, result.venue_order);
                hash_cancel_outcome(hasher, result.outcome);
                hash_cancel_reject_reason(hasher, result.reject_reason);
            }
            PmJournalRecordV1::FillApplied(applied) => {
                let fill = applied.fill;
                self.hash_fill_key(hasher, fill.key);
                self.hash_client(hasher, fill.client_order);
                hash_instrument(hasher, fill.instrument);
                hash_side(hasher, fill.side);
                hasher.update(fill.price_units.to_be_bytes());
                hash_fill_role(hasher, fill.role);
                hash_fill_settlement(hasher, fill.settlement);
                hash_fill_fee(hasher, fill.fee);
                hash_quantity(hasher, fill.delta);
                hash_optional_u256(hasher, fill.authoritative_cumulative);
                hash_u256(hasher, fill.cumulative);
                hash_u256(hasher, fill.remaining);
                hash_fill_source(hasher, applied.source);
                self.hash_occurrence(hasher, applied.occurrence);
                hash_fill_delivery(hasher, applied.delivery);
            }
            PmJournalRecordV1::OrderTerminal(terminal) => {
                self.hash_client(hasher, terminal.client_order);
                self.hash_venue(hasher, terminal.venue_order);
                hash_terminal_status(hasher, terminal.status);
                hash_u256(hasher, terminal.cumulative);
                hash_u256(hasher, terminal.remaining);
                hash_order_progress_source(hasher, terminal.source);
                self.hash_occurrence(hasher, terminal.occurrence);
            }
            PmJournalRecordV1::SafetyHalt(halt) => {
                hasher.update(halt.account.ordinal().to_be_bytes());
                hash_safety_reason(hasher, halt.reason);
            }
            PmJournalRecordV1::FillWatermarkAdvanced(watermark) => {
                self.watermark_ordinal = self.watermark_ordinal.saturating_add(1);
                self.valid &= self
                    .previous_watermark
                    .is_none_or(|previous| previous != watermark.cursor.opaque);
                self.previous_watermark = Some(watermark.cursor.opaque);
                hash_account_scope(hasher, watermark.cursor.account_scope);
                hasher.update(self.watermark_ordinal.to_be_bytes());
            }
        }
    }

    fn hash_client(&mut self, hasher: &mut Sha256, client: PmClientOrderKey) {
        self.valid &= Some(client) == self.current_client;
        hasher.update(client.account().ordinal().to_be_bytes());
        hasher.update(self.quote_ordinal.to_be_bytes());
    }

    fn hash_optional_venue(&mut self, hasher: &mut Sha256, venue: Option<PmVenueOrderKey>) {
        match venue {
            Some(venue) => {
                hasher.update([1]);
                self.hash_venue(hasher, venue);
            }
            None => hasher.update([0]),
        }
    }

    fn hash_venue(&mut self, hasher: &mut Sha256, venue: PmVenueOrderKey) {
        hasher.update(venue.account().ordinal().to_be_bytes());
        let ordinal = normalized_fixture_ordinal(
            venue.id().as_str(),
            "phase6-venue-",
            self.quote_ordinal,
            &mut self.venue_base,
            &mut self.valid,
        );
        hasher.update(ordinal.to_be_bytes());
    }

    fn hash_fill_key(&mut self, hasher: &mut Sha256, key: PmJournalFillKeyV1) {
        self.hash_venue(hasher, key.venue_order);
        let ordinal = normalized_fill_ordinal(
            key.fill_id,
            self.quote_ordinal,
            &mut self.fill_base,
            &mut self.valid,
        );
        hasher.update(ordinal.to_be_bytes());
    }

    fn hash_occurrence(&mut self, hasher: &mut Sha256, occurrence: PmJournalFillOccurrenceV1) {
        hasher.update(
            relative(
                &mut self.occurrence_bases[0],
                occurrence.owner_sequence.value(),
                &mut self.valid,
            )
            .to_be_bytes(),
        );
        hash_optional_connection(hasher, occurrence.connection);
        hash_optional_u64(
            hasher,
            occurrence.connection_epoch.map(|epoch| epoch.value()),
        );
        hasher.update(
            occurrence
                .ingress_sequence
                .map_or(0, |value| {
                    relative(
                        &mut self.occurrence_bases[1],
                        value.value(),
                        &mut self.valid,
                    )
                })
                .to_be_bytes(),
        );
        hasher.update(
            occurrence
                .snapshot_revision
                .map_or(0, |value| {
                    relative(
                        &mut self.occurrence_bases[2],
                        value.value(),
                        &mut self.valid,
                    )
                })
                .to_be_bytes(),
        );
        hasher.update(
            relative(
                &mut self.occurrence_bases[3],
                occurrence.monotonic_service_ns,
                &mut self.valid,
            )
            .to_be_bytes(),
        );
    }
}

fn relative(base: &mut Option<u64>, value: u64, valid: &mut bool) -> u64 {
    let base = *base.get_or_insert(value);
    *valid &= value >= base;
    value.saturating_sub(base).saturating_add(1)
}

fn normalized_fixture_ordinal(
    value: &str,
    prefix: &str,
    expected_ordinal: u64,
    base: &mut Option<u64>,
    valid: &mut bool,
) -> u64 {
    let suffix = value.strip_prefix(prefix);
    *valid &= suffix.is_some_and(|value| value.len() == 6);
    let absolute = suffix.and_then(|value| value.parse::<u64>().ok());
    let Some(absolute) = absolute else {
        *valid = false;
        return 0;
    };
    let Some(candidate_base) = absolute.checked_sub(expected_ordinal) else {
        *valid = false;
        return expected_ordinal;
    };
    let observed_base = *base.get_or_insert(candidate_base);
    *valid &= candidate_base == observed_base;
    expected_ordinal
}

fn normalized_fill_ordinal(
    fill: PmFillId,
    expected_ordinal: u64,
    base: &mut Option<u64>,
    valid: &mut bool,
) -> u64 {
    normalized_fixture_ordinal(fill.as_str(), "phase6-fill-", expected_ordinal, base, valid)
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).expect("in-memory byte slice length fits u64");
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
}

fn hash_u256(hasher: &mut Sha256, value: U256) {
    hasher.update(value.to_be_bytes());
}

fn hash_optional_u256(hasher: &mut Sha256, value: Option<U256>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_u256(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        None => hasher.update([0]),
    }
}

fn hash_quantity(hasher: &mut Sha256, value: PmQuantity) {
    hash_u256(hasher, value.protocol_units());
}

fn hash_address(hasher: &mut Sha256, value: EvmAddress) {
    hasher.update(value.bytes());
}

fn hash_instrument(hasher: &mut Sha256, value: PmInstrumentId) {
    hasher.update(value.market().bytes());
    hash_u256(hasher, value.token().units());
}

fn hash_account_scope(hasher: &mut Sha256, value: PmAccountScope) {
    hash_bytes(hasher, value.environment().as_str().as_bytes());
    hasher.update(value.chain().value().to_be_bytes());
    hash_address(hasher, value.signer().address());
    hash_address(hasher, value.funder().address());
    hasher.update(value.handle().ordinal().to_be_bytes());
}

fn hash_optional_connection(hasher: &mut Sha256, value: Option<PmConnectionId>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_bytes(hasher, value.as_str().as_bytes());
        }
        None => hasher.update([0]),
    }
}

fn hash_asset(hasher: &mut Sha256, value: PmAssetId) {
    match value {
        PmAssetId::Collateral { contract } => {
            hasher.update([0]);
            hash_address(hasher, contract);
        }
        PmAssetId::Outcome { contract, token } => {
            hasher.update([1]);
            hash_address(hasher, contract);
            hash_u256(hasher, token.units());
        }
    }
}

fn hash_side(hasher: &mut Sha256, value: PmJournalSideV1) {
    hasher.update([match value {
        PmJournalSideV1::Buy => 0,
        PmJournalSideV1::Sell => 1,
    }]);
}

fn hash_profile(hasher: &mut Sha256, value: PmJournalQuoteProfileV1) {
    hasher.update([match value {
        PmJournalQuoteProfileV1::PassiveGtcPostOnlyEoa => 0,
    }]);
}

fn hash_place_outcome(hasher: &mut Sha256, value: PmJournalPlaceOutcomeV1) {
    hasher.update([match value {
        PmJournalPlaceOutcomeV1::AcceptedResting => 0,
        PmJournalPlaceOutcomeV1::AcceptedWithImmediateFill => 1,
        PmJournalPlaceOutcomeV1::Rejected => 2,
        PmJournalPlaceOutcomeV1::AmbiguousTimeout => 3,
        PmJournalPlaceOutcomeV1::LateAcknowledgement => 4,
    }]);
}

fn hash_place_reject_reason(hasher: &mut Sha256, value: Option<PmJournalPlaceRejectReasonV1>) {
    hasher.update([match value {
        None => 0,
        Some(PmJournalPlaceRejectReasonV1::FixtureRejected) => 1,
        Some(PmJournalPlaceRejectReasonV1::PostOnlyWouldTake) => 2,
        Some(PmJournalPlaceRejectReasonV1::AuthorityInvalidatedBeforeDispatch) => 3,
    }]);
}

fn hash_cancel_reason(hasher: &mut Sha256, value: PmJournalCancelReasonV1) {
    hasher.update([match value {
        PmJournalCancelReasonV1::Replacement => 0,
        PmJournalCancelReasonV1::StaleReference => 1,
        PmJournalCancelReasonV1::StaleBook => 2,
        PmJournalCancelReasonV1::SafetyHalt => 3,
    }]);
}

fn hash_cancel_outcome(hasher: &mut Sha256, value: PmJournalCancelOutcomeV1) {
    hasher.update([match value {
        PmJournalCancelOutcomeV1::Accepted => 0,
        PmJournalCancelOutcomeV1::Rejected => 1,
        PmJournalCancelOutcomeV1::AlreadyFilled => 2,
        PmJournalCancelOutcomeV1::AmbiguousTimeout => 3,
    }]);
}

fn hash_cancel_reject_reason(hasher: &mut Sha256, value: Option<PmJournalCancelRejectReasonV1>) {
    hasher.update([match value {
        None => 0,
        Some(PmJournalCancelRejectReasonV1::FixtureRejected) => 1,
    }]);
}

fn hash_fill_role(hasher: &mut Sha256, value: PmJournalFillRoleV1) {
    hasher.update([match value {
        PmJournalFillRoleV1::Maker => 0,
        PmJournalFillRoleV1::Taker => 1,
    }]);
}

fn hash_fill_settlement(hasher: &mut Sha256, value: PmJournalFillSettlementV1) {
    hasher.update([match value {
        PmJournalFillSettlementV1::Matched => 0,
        PmJournalFillSettlementV1::Mined => 1,
        PmJournalFillSettlementV1::Confirmed => 2,
        PmJournalFillSettlementV1::Retrying => 3,
        PmJournalFillSettlementV1::Failed => 4,
    }]);
}

fn hash_fill_fee(hasher: &mut Sha256, value: PmJournalFillFeeV1) {
    match value {
        PmJournalFillFeeV1::Known {
            asset,
            sign,
            magnitude,
        } => {
            hasher.update([0]);
            hash_asset(hasher, asset);
            hasher.update([match sign {
                PmJournalSignV1::Positive => 0,
                PmJournalSignV1::Negative => 1,
            }]);
            hash_u256(hasher, magnitude);
        }
        PmJournalFillFeeV1::Unknown => hasher.update([1]),
        PmJournalFillFeeV1::Incomplete => hasher.update([2]),
    }
}

fn hash_fill_source(hasher: &mut Sha256, value: PmJournalFillSourceV1) {
    hasher.update([match value {
        PmJournalFillSourceV1::PlaceAcknowledgement => 0,
        PmJournalFillSourceV1::PrivateWebsocket => 1,
        PmJournalFillSourceV1::RestReconciliation => 2,
    }]);
}

fn hash_fill_delivery(hasher: &mut Sha256, value: PmJournalFillDeliveryV1) {
    hasher.update([match value {
        PmJournalFillDeliveryV1::Live => 0,
        PmJournalFillDeliveryV1::Replay => 1,
    }]);
}

fn hash_terminal_status(hasher: &mut Sha256, value: PmJournalTerminalStatusV1) {
    hasher.update([match value {
        PmJournalTerminalStatusV1::Filled => 0,
        PmJournalTerminalStatusV1::Cancelled => 1,
        PmJournalTerminalStatusV1::Rejected => 2,
        PmJournalTerminalStatusV1::Expired => 3,
    }]);
}

fn hash_order_progress_source(hasher: &mut Sha256, value: PmJournalOrderProgressSourceV1) {
    hasher.update([match value {
        PmJournalOrderProgressSourceV1::PrivateWebsocket => 0,
        PmJournalOrderProgressSourceV1::RestReconciliation => 1,
    }]);
}

fn hash_safety_reason(hasher: &mut Sha256, value: PmJournalSafetyReasonV1) {
    hasher.update([match value {
        PmJournalSafetyReasonV1::ContractViolation => 0,
        PmJournalSafetyReasonV1::UnresolvedOwnership => 1,
        PmJournalSafetyReasonV1::DurableWriteFailure => 2,
        PmJournalSafetyReasonV1::QueueSaturation => 3,
        PmJournalSafetyReasonV1::StaleDependency => 4,
    }]);
}

struct ArrayVec64 {
    len: u8,
    values: [Option<PmJournalFillKeyV1>; 64],
}

impl FromIterator<PmJournalFillKeyV1> for ArrayVec64 {
    fn from_iter<T: IntoIterator<Item = PmJournalFillKeyV1>>(iter: T) -> Self {
        let mut output = Self {
            len: 0,
            values: [None; 64],
        };
        for value in iter {
            let index = usize::from(output.len);
            if index == output.values.len() {
                break;
            }
            output.values[index] = Some(value);
            output.len += 1;
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use reap_pm_core::{
        ConnectionEpoch, IngressSequence, PmConnectionId, PmFillId, PmOrderSalt, PmOrderSide,
        PmPrice, PmQuantity, PmSignedUnits, PmVenueOrderId, PmVenueOrderKey, SnapshotRevision,
        exact_order_amounts,
    };

    use super::*;
    use crate::journal::schema::{
        PmJournalCancelIntentV1, PmJournalCancelResultV1, PmJournalFillAppliedV1,
        PmJournalFillCursorV1, PmJournalFillV1, PmJournalFillWatermarkV1, PmJournalHeaderV1,
        PmJournalImmediateFillsV1, PmJournalOrderTerminalV1, PmJournalPlaceResultV1,
        PmJournalQuoteIntentV1, PmJournalSafetyHaltV1, PmJournalScopeV1,
        derive_pm_journal_client_order, test_scope,
    };

    const WALL_BASE_NS: u64 = 1_700_000_000_000_000_000;

    fn normalized_records(scope: &PmJournalScopeV1, base: u64) -> Vec<PmJournalRecordV1> {
        let intent_id = base + 1;
        let price = PmPrice::from_units(500_000).expect("price");
        let quantity = PmQuantity::parse_decimal("1").expect("quantity");
        let amounts =
            exact_order_amounts(PmOrderSide::Buy, price, quantity).expect("order amounts");
        let account_scope = scope.account_scope();
        let intent = PmJournalQuoteIntentV1 {
            intent_id,
            client_order: derive_pm_journal_client_order(scope, intent_id).expect("client order"),
            instrument: scope.instrument(),
            side: PmJournalSideV1::Buy,
            price_units: price.units(),
            quantity,
            reserved_collateral: amounts.maker(),
            reserved_outcome: U256::ZERO,
            profile: PmJournalQuoteProfileV1::PassiveGtcPostOnlyEoa,
            metadata_revision: base + 2,
            book_revision: base + 3,
            model_revision: base + 4,
            book_readiness_revision: base + 5,
            private_readiness_revision: base + 6,
            expires_at_monotonic_ns: base + 7,
            salt: PmOrderSalt::from_u64(base + 8).expect("salt"),
            timestamp_ms: base + 9,
            maker: account_scope.funder().address(),
            signer: account_scope.signer().address(),
            maker_amount: amounts.maker(),
            taker_amount: amounts.taker(),
        };
        let venue = PmVenueOrderKey::new(
            scope.account(),
            PmVenueOrderId::new(&format!("phase6-venue-{:06}", base + 1)).expect("venue order"),
        );
        let fill_key = PmJournalFillKeyV1 {
            venue_order: venue,
            fill_id: PmFillId::new(&format!("phase6-fill-{:06}", base + 1)).expect("fill id"),
        };
        let occurrence = PmJournalFillOccurrenceV1 {
            owner_sequence: IngressSequence::new(base + 10),
            connection: Some(PmConnectionId::new("pm-private").expect("connection")),
            connection_epoch: Some(ConnectionEpoch::new(7)),
            ingress_sequence: Some(IngressSequence::new(base + 12)),
            snapshot_revision: Some(SnapshotRevision::new(base + 13)),
            monotonic_service_ns: base + 14,
        };
        let fill = PmJournalFillAppliedV1 {
            fill: PmJournalFillV1 {
                key: fill_key,
                client_order: intent.client_order,
                instrument: intent.instrument,
                side: intent.side,
                price_units: intent.price_units,
                role: PmJournalFillRoleV1::Maker,
                settlement: PmJournalFillSettlementV1::Matched,
                fee: PmJournalFillFeeV1::Known {
                    asset: PmAssetId::collateral(account_scope.funder().address()),
                    sign: PmJournalSignV1::Positive,
                    magnitude: PmSignedUnits::ZERO.magnitude(),
                },
                delta: quantity,
                authoritative_cumulative: Some(quantity.protocol_units()),
                cumulative: quantity.protocol_units(),
                remaining: U256::ZERO,
            },
            source: PmJournalFillSourceV1::RestReconciliation,
            occurrence,
            delivery: PmJournalFillDeliveryV1::Live,
        };
        vec![
            PmJournalRecordV1::Header(PmJournalHeaderV1::new(scope.clone())),
            PmJournalRecordV1::QuoteIntent(intent),
            PmJournalRecordV1::PlaceResult(PmJournalPlaceResultV1 {
                client_order: intent.client_order,
                outcome: PmJournalPlaceOutcomeV1::AcceptedResting,
                reject_reason: None,
                venue_order: Some(venue),
                immediate_fills: PmJournalImmediateFillsV1::empty(),
            }),
            PmJournalRecordV1::CancelIntent(PmJournalCancelIntentV1 {
                client_order: intent.client_order,
                venue_order: venue,
                reason: PmJournalCancelReasonV1::Replacement,
            }),
            PmJournalRecordV1::CancelResult(PmJournalCancelResultV1 {
                client_order: intent.client_order,
                venue_order: venue,
                outcome: PmJournalCancelOutcomeV1::Accepted,
                reject_reason: None,
            }),
            PmJournalRecordV1::FillApplied(fill),
            PmJournalRecordV1::OrderTerminal(PmJournalOrderTerminalV1 {
                client_order: intent.client_order,
                venue_order: venue,
                status: PmJournalTerminalStatusV1::Filled,
                cumulative: quantity.protocol_units(),
                remaining: U256::ZERO,
                source: PmJournalOrderProgressSourceV1::RestReconciliation,
                occurrence,
            }),
            PmJournalRecordV1::SafetyHalt(PmJournalSafetyHaltV1 {
                account: scope.account(),
                reason: PmJournalSafetyReasonV1::StaleDependency,
            }),
            PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 {
                cursor: PmJournalFillCursorV1 {
                    account_scope,
                    opaque: PmJournalFingerprintV1::from_bytes([base as u8; 32]),
                },
            }),
        ]
    }

    fn hash_records(
        scope: &PmJournalScopeV1,
        sequence_base: u64,
        records: &[PmJournalRecordV1],
    ) -> [u8; 32] {
        let mut segment = PmSealedJournalSegment::new(scope.account(), scope.fingerprint());
        for (offset, record) in records.iter().enumerate() {
            segment.observe(
                sequence_base + offset as u64,
                crate::journal::sealed_record_index(record),
                record,
            );
        }
        assert!(segment.valid(), "fixed normalized corpus must remain valid");
        segment.hash()
    }

    fn quote_records_for_pass(
        scope: &PmJournalScopeV1,
        identity_base: u64,
        monotonic_start_ns: u64,
    ) -> Vec<PmJournalRecordV1> {
        let price = PmPrice::from_units(500_000).expect("price");
        let quantity = PmQuantity::parse_decimal("1").expect("quantity");
        let amounts =
            exact_order_amounts(PmOrderSide::Buy, price, quantity).expect("order amounts");
        let account_scope = scope.account_scope();
        (0..10)
            .map(|offset| {
                let ordinal = u64::try_from(offset).expect("fixed quote ordinal");
                let identity = identity_base + ordinal + 1;
                let monotonic_ns = monotonic_start_ns + ordinal * 175_010;
                PmJournalRecordV1::QuoteIntent(PmJournalQuoteIntentV1 {
                    intent_id: identity,
                    client_order: derive_pm_journal_client_order(scope, identity)
                        .expect("client order"),
                    instrument: scope.instrument(),
                    side: PmJournalSideV1::Buy,
                    price_units: price.units(),
                    quantity,
                    reserved_collateral: amounts.maker(),
                    reserved_outcome: U256::ZERO,
                    profile: PmJournalQuoteProfileV1::PassiveGtcPostOnlyEoa,
                    metadata_revision: identity_base + ordinal + 20,
                    book_revision: identity_base + ordinal + 30,
                    model_revision: identity_base + ordinal + 40,
                    book_readiness_revision: identity_base + ordinal + 50,
                    private_readiness_revision: identity_base + ordinal + 60,
                    expires_at_monotonic_ns: monotonic_ns + 1_000_000,
                    salt: PmOrderSalt::from_u64(identity_base + ordinal + 70).expect("salt"),
                    timestamp_ms: (WALL_BASE_NS + monotonic_ns) / 1_000_000,
                    maker: account_scope.funder().address(),
                    signer: account_scope.signer().address(),
                    maker_amount: amounts.maker(),
                    taker_amount: amounts.taker(),
                })
            })
            .collect()
    }

    #[test]
    fn normalized_artifacts_with_shifted_absolute_identities_hash_equally() {
        let scope = test_scope();
        assert_eq!(
            hash_records(&scope, 1_000, &normalized_records(&scope, 10)),
            hash_records(&scope, 9_000, &normalized_records(&scope, 20)),
        );
    }

    #[test]
    fn aligned_pass_timestamp_buckets_hash_equally_and_bucket_phase_is_material() {
        let scope = test_scope();
        let aligned_first = quote_records_for_pass(&scope, 10, 1_000);
        let aligned_shifted = quote_records_for_pass(&scope, 20, 2_001_000);
        let misaligned_shifted = quote_records_for_pass(&scope, 20, 1_751_100);

        let expected = hash_records(&scope, 1_000, &aligned_first);
        assert_eq!(
            hash_records(&scope, 9_000, &aligned_shifted),
            expected,
            "an integer-millisecond pass shift preserves every relative timestamp bucket"
        );
        assert_ne!(
            hash_records(&scope, 9_000, &misaligned_shifted),
            expected,
            "a changed timestamp bucket-transition position remains material to the hash"
        );
    }

    #[test]
    fn segment_hash_is_versioned_and_scope_seeded_even_without_records() {
        let scope = test_scope();
        let first = PmSealedJournalSegment::new(
            scope.account(),
            PmJournalFingerprintV1::from_bytes([1; 32]),
        )
        .hash();
        let second = PmSealedJournalSegment::new(
            scope.account(),
            PmJournalFingerprintV1::from_bytes([2; 32]),
        )
        .hash();
        assert_ne!(first, second, "scope fingerprint must seed every segment");
        assert_ne!(
            first,
            Sha256::new().finalize().as_slice(),
            "the versioned segment domain must seed an otherwise empty segment"
        );
    }

    #[test]
    fn byte_lengths_use_fixed_u64_encoding() {
        let bytes = b"scope";
        let mut actual = Sha256::new();
        hash_bytes(&mut actual, bytes);
        let mut expected = Sha256::new();
        expected.update(
            u64::try_from(bytes.len())
                .expect("fixed byte length")
                .to_be_bytes(),
        );
        expected.update(bytes);
        assert_eq!(actual.finalize(), expected.finalize());
    }

    #[test]
    fn fixture_identity_underflow_is_invalid_for_venue_and_fill() {
        let mut venue_base = None;
        let mut venue_valid = true;
        assert_eq!(
            normalized_fixture_ordinal(
                "phase6-venue-000001",
                "phase6-venue-",
                2,
                &mut venue_base,
                &mut venue_valid,
            ),
            2
        );
        assert!(!venue_valid);
        assert_eq!(venue_base, None);

        let mut fill_base = None;
        let mut fill_valid = true;
        assert_eq!(
            normalized_fill_ordinal(
                PmFillId::new("phase6-fill-000001").expect("fill id"),
                2,
                &mut fill_base,
                &mut fill_valid,
            ),
            2
        );
        assert!(!fill_valid);
        assert_eq!(fill_base, None);
    }

    #[test]
    fn segment_sequence_must_be_contiguous_after_normalization() {
        let scope = test_scope();
        let record = PmJournalRecordV1::SafetyHalt(PmJournalSafetyHaltV1 {
            account: scope.account(),
            reason: PmJournalSafetyReasonV1::StaleDependency,
        });

        let mut contiguous = PmSealedJournalSegment::new(scope.account(), scope.fingerprint());
        contiguous.observe(100, crate::journal::sealed_record_index(&record), &record);
        contiguous.observe(101, crate::journal::sealed_record_index(&record), &record);
        assert!(contiguous.valid());

        let mut gap = PmSealedJournalSegment::new(scope.account(), scope.fingerprint());
        gap.observe(100, crate::journal::sealed_record_index(&record), &record);
        gap.observe(102, crate::journal::sealed_record_index(&record), &record);
        assert!(!gap.valid());

        let mut duplicate = PmSealedJournalSegment::new(scope.account(), scope.fingerprint());
        duplicate.observe(100, crate::journal::sealed_record_index(&record), &record);
        duplicate.observe(100, crate::journal::sealed_record_index(&record), &record);
        assert!(!duplicate.valid());
    }

    #[test]
    fn every_record_variant_contributes_a_distinct_segment_hash() {
        let scope = test_scope();
        let records = normalized_records(&scope, 10);
        let hashes = records
            .iter()
            .map(|record| {
                let mut segment = PmSealedJournalSegment::new(scope.account(), scope.fingerprint());
                segment.observe(1, crate::journal::sealed_record_index(record), record);
                segment.hash()
            })
            .collect::<HashSet<_>>();
        assert_eq!(hashes.len(), records.len());
    }

    #[test]
    fn material_quote_and_fill_field_changes_change_the_segment_hash() {
        let scope = test_scope();
        let baseline = normalized_records(&scope, 10);
        let expected = hash_records(&scope, 100, &baseline);

        let mut changed_quote = baseline.clone();
        let PmJournalRecordV1::QuoteIntent(intent) = &mut changed_quote[1] else {
            unreachable!("fixed quote record");
        };
        intent.price_units += 1;
        assert_ne!(hash_records(&scope, 100, &changed_quote), expected);

        let mut changed_fill = baseline.clone();
        let PmJournalRecordV1::FillApplied(applied) = &mut changed_fill[5] else {
            unreachable!("fixed fill record");
        };
        applied.fill.settlement = PmJournalFillSettlementV1::Confirmed;
        assert_ne!(hash_records(&scope, 100, &changed_fill), expected);
    }
}
