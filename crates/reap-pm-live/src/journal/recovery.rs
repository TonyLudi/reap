use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

use reap_pm_core::{PmClientOrderKey, PmVenueOrderKey, U256};
use thiserror::Error;

use super::schema::{
    MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS, MAX_PM_JOURNAL_BYTES, MAX_PM_JOURNAL_FILL_KEYS,
    MAX_PM_JOURNAL_LINE_BYTES, MAX_PM_JOURNAL_OWNED_ORDERS, MAX_PM_JOURNAL_RECORDS,
    PmJournalCancelOutcomeV1, PmJournalFillAppliedV1, PmJournalFillCursorV1, PmJournalFillKeyV1,
    PmJournalFillOccurrenceV1, PmJournalFillSourceV1, PmJournalImmediateFillsV1, PmJournalLineV1,
    PmJournalPlaceOutcomeV1, PmJournalQuoteIntentV1, PmJournalRecordV1, PmJournalSafetyReasonV1,
    PmJournalScopeV1, PmJournalTerminalStatusV1, next_sequence,
};

/// Bounded deterministic projection recovered from one checked PM journal.
///
/// This is observation-only evidence. Executable authority is reconstructed
/// only by the crate-private coordinator after a fresh complete
/// reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmJournalRecovery {
    scope: PmJournalScopeV1,
    record_count: usize,
    last_sequence: u64,
    last_intent_id: u64,
    last_owned_observation_sequence: u64,
    compacted_intent_id: u64,
    owned_orders: BTreeMap<PmClientOrderKey, RecoveredOwnedOrder>,
    order_order: BTreeMap<u64, PmClientOrderKey>,
    fill_keys: BTreeMap<PmJournalFillKeyV1, PmJournalFillAppliedV1>,
    fill_order: BTreeMap<u64, PmJournalFillKeyV1>,
    observation_order: BTreeMap<u64, PmJournalRecoveredObservationV1>,
    safety_halt: Option<PmJournalSafetyReasonV1>,
    fill_watermark: Option<PmJournalFillCursorV1>,
}

impl PmJournalRecovery {
    fn empty(scope: PmJournalScopeV1) -> Self {
        Self {
            scope,
            record_count: 0,
            last_sequence: 0,
            last_intent_id: 0,
            last_owned_observation_sequence: 0,
            compacted_intent_id: 0,
            owned_orders: BTreeMap::new(),
            order_order: BTreeMap::new(),
            fill_keys: BTreeMap::new(),
            fill_order: BTreeMap::new(),
            observation_order: BTreeMap::new(),
            safety_halt: None,
            fill_watermark: None,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> &PmJournalScopeV1 {
        &self.scope
    }

    #[must_use]
    pub const fn record_count(&self) -> usize {
        self.record_count
    }

    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    #[must_use]
    pub const fn last_intent_id(&self) -> u64 {
        self.last_intent_id
    }

    #[must_use]
    pub const fn last_owned_observation_sequence(&self) -> u64 {
        self.last_owned_observation_sequence
    }

    #[must_use]
    pub const fn compacted_intent_id(&self) -> u64 {
        self.compacted_intent_id
    }

    #[must_use]
    pub fn owned_order_count(&self) -> usize {
        self.owned_orders.len()
    }

    #[must_use]
    pub fn fill_key_count(&self) -> usize {
        self.fill_keys.len()
    }

    #[must_use]
    pub const fn safety_halted(&self) -> bool {
        self.safety_halt.is_some()
    }

    #[must_use]
    pub const fn safety_reason(&self) -> Option<PmJournalSafetyReasonV1> {
        self.safety_halt
    }

    #[must_use]
    pub const fn fill_watermark(&self) -> Option<PmJournalFillCursorV1> {
        self.fill_watermark
    }

    #[must_use]
    pub fn unresolved_order_count(&self) -> usize {
        self.owned_orders
            .values()
            .filter(|order| order.is_unresolved())
            .count()
    }

    /// Whether journal evidence must be reconciled before executable
    /// authority can be reconstructed.
    #[must_use]
    pub fn requires_reconciliation(&self) -> bool {
        self.safety_halt.is_some() || !self.owned_orders.is_empty() || !self.fill_keys.is_empty()
    }

    /// Intent-identity ordered, copy-only bootstrap observations.
    pub(crate) fn recovered_orders(
        &self,
    ) -> impl DoubleEndedIterator<Item = PmJournalRecoveredOrderV1> + ExactSizeIterator + '_ {
        self.order_order.values().map(|client| {
            self.owned_orders
                .get(client)
                .expect("intent-order index must reference retained order")
                .bootstrap_row()
        })
    }

    /// Coordinator-owner ordered full fill facts for canonical bootstrap.
    ///
    /// Recovery retains the exact execution, fee, settlement, source and
    /// occurrence evidence rather than asking state to synthesize those facts
    /// from an aggregate cumulative total.
    #[cfg(test)]
    pub(crate) fn recovered_fills(
        &self,
    ) -> impl DoubleEndedIterator<Item = PmJournalFillAppliedV1> + ExactSizeIterator + '_ {
        self.fill_order.values().map(|key| {
            *self
                .fill_keys
                .get(key)
                .expect("fill-order index must reference retained fill")
        })
    }

    /// Coordinator-owner ordered durable observations for exact bootstrap.
    ///
    /// Fill and terminal-progress facts share one owner sequence. Replaying
    /// separate type buckets would reverse valid cancel/fill races.
    pub(crate) fn recovered_observations(
        &self,
    ) -> impl DoubleEndedIterator<Item = PmJournalRecoveredObservationV1> + ExactSizeIterator + '_
    {
        self.observation_order.values().copied()
    }

    #[cfg(test)]
    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.owned_orders.len() * std::mem::size_of::<RecoveredOwnedOrder>()
            + self.order_order.len()
                * (std::mem::size_of::<u64>() + std::mem::size_of::<PmClientOrderKey>())
            + self.fill_keys.len()
                * (std::mem::size_of::<PmJournalFillKeyV1>()
                    + std::mem::size_of::<PmJournalFillAppliedV1>())
            + self.fill_order.len()
                * (std::mem::size_of::<u64>() + std::mem::size_of::<PmJournalFillKeyV1>())
            + self.observation_order.len()
                * (std::mem::size_of::<u64>()
                    + std::mem::size_of::<PmJournalRecoveredObservationV1>())
    }
}

/// One exact mutation-owner observation retained for canonical bootstrap.
#[allow(
    clippy::large_enum_variant,
    reason = "replay observations preserve exact journal evidence inline without per-record allocation"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmJournalRecoveredObservationV1 {
    FillApplied(PmJournalFillAppliedV1),
    OrderTerminal(super::schema::PmJournalOrderTerminalV1),
}

impl PmJournalRecoveredObservationV1 {
    const fn client_order(self) -> PmClientOrderKey {
        match self {
            Self::FillApplied(applied) => applied.fill.client_order,
            Self::OrderTerminal(terminal) => terminal.client_order,
        }
    }
}

/// Observation-only place state retained by checked journal recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmJournalRecoveredPlaceV1 {
    IntentOnly,
    Unknown,
    Bound,
    Rejected,
}

/// One bounded, copy-only order row for canonical state-owner bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmJournalRecoveredOrderV1 {
    intent: PmJournalQuoteIntentV1,
    venue_order: Option<PmVenueOrderKey>,
    place: PmJournalRecoveredPlaceV1,
    known_fill_total: U256,
    authoritative_cumulative: Option<U256>,
    cumulative: U256,
    cancel_pending: bool,
    cancel_unknown: bool,
    pending_ack_fill_count: u8,
    terminal: Option<PmJournalTerminalStatusV1>,
}

impl PmJournalRecoveredOrderV1 {
    #[must_use]
    pub(crate) const fn intent(self) -> PmJournalQuoteIntentV1 {
        self.intent
    }

    #[must_use]
    pub(crate) const fn venue_order(self) -> Option<PmVenueOrderKey> {
        self.venue_order
    }

    #[must_use]
    pub(crate) const fn place(self) -> PmJournalRecoveredPlaceV1 {
        self.place
    }

    #[must_use]
    pub(crate) const fn known_fill_total(self) -> U256 {
        self.known_fill_total
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn authoritative_cumulative(self) -> Option<U256> {
        self.authoritative_cumulative
    }

    #[must_use]
    pub(crate) const fn effective_cumulative(self) -> U256 {
        self.cumulative
    }

    #[must_use]
    pub(crate) const fn cancel_pending(self) -> bool {
        self.cancel_pending
    }

    #[must_use]
    pub(crate) const fn cancel_unknown(self) -> bool {
        self.cancel_unknown
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn pending_ack_fill_count(self) -> u8 {
        self.pending_ack_fill_count
    }

    #[must_use]
    pub(crate) const fn terminal(self) -> Option<PmJournalTerminalStatusV1> {
        self.terminal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveredPlace {
    IntentOnly,
    Unknown,
    Bound,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactionProof {
    Automatic,
    SlotReplacementOrTerminal,
    FillWatermark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingAcknowledgementFills {
    entries: [Option<PmJournalFillKeyV1>; MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS],
    count: u8,
}

impl PendingAcknowledgementFills {
    const fn empty() -> Self {
        Self {
            entries: [None; MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS],
            count: 0,
        }
    }

    fn replace_from(&mut self, fills: &PmJournalImmediateFillsV1) {
        *self = Self::empty();
        for key in fills.iter() {
            let index = usize::from(self.count);
            self.entries[index] = Some(key);
            self.count += 1;
        }
    }

    fn resolve(&mut self, key: PmJournalFillKeyV1) -> bool {
        let Some(index) = self.entries.iter().position(|entry| *entry == Some(key)) else {
            return false;
        };
        self.entries[index] = None;
        self.count -= 1;
        true
    }

    fn contains(&self, key: PmJournalFillKeyV1) -> bool {
        self.entries.contains(&Some(key))
    }

    const fn is_empty(self) -> bool {
        self.count == 0
    }

    const fn len(self) -> u8 {
        self.count
    }
}

/// Exact non-authoritative order facts retained for coordinator bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecoveredOwnedOrder {
    intent: PmJournalQuoteIntentV1,
    known_fill_total: U256,
    authoritative_cumulative: Option<U256>,
    cumulative: U256,
    last_authoritative_occurrence: Option<RecoveredAuthoritativeOccurrence>,
    venue_order: Option<PmVenueOrderKey>,
    place: RecoveredPlace,
    cancel_pending: bool,
    cancel_unknown: bool,
    pending_ack_fills: PendingAcknowledgementFills,
    terminal: Option<PmJournalTerminalStatusV1>,
}

impl RecoveredOwnedOrder {
    fn bootstrap_row(&self) -> PmJournalRecoveredOrderV1 {
        PmJournalRecoveredOrderV1 {
            intent: self.intent,
            venue_order: self.venue_order,
            place: match self.place {
                RecoveredPlace::IntentOnly => PmJournalRecoveredPlaceV1::IntentOnly,
                RecoveredPlace::Unknown => PmJournalRecoveredPlaceV1::Unknown,
                RecoveredPlace::Bound => PmJournalRecoveredPlaceV1::Bound,
                RecoveredPlace::Rejected => PmJournalRecoveredPlaceV1::Rejected,
            },
            known_fill_total: self.known_fill_total,
            authoritative_cumulative: self.authoritative_cumulative,
            cumulative: self.cumulative,
            cancel_pending: self.cancel_pending,
            cancel_unknown: self.cancel_unknown,
            pending_ack_fill_count: self.pending_ack_fills.len(),
            terminal: self.terminal,
        }
    }

    fn original(self) -> U256 {
        self.intent.quantity.protocol_units()
    }

    fn is_unresolved(&self) -> bool {
        self.place == RecoveredPlace::Unknown
            || self.cancel_pending
            || self.cancel_unknown
            || !self.pending_ack_fills.is_empty()
            || self.known_fill_total != self.cumulative
    }

    fn can_compact(self, proof: CompactionProof) -> bool {
        if self.is_unresolved() {
            return false;
        }
        let fill_bearing = !self.known_fill_total.is_zero();
        match self.terminal {
            Some(PmJournalTerminalStatusV1::Filled) => {
                !fill_bearing || proof == CompactionProof::FillWatermark
            }
            Some(PmJournalTerminalStatusV1::Rejected) => {
                (self.place == RecoveredPlace::Rejected || proof != CompactionProof::Automatic)
                    && (!fill_bearing || proof == CompactionProof::FillWatermark)
            }
            Some(PmJournalTerminalStatusV1::Cancelled | PmJournalTerminalStatusV1::Expired) => {
                proof != CompactionProof::Automatic
                    && (!fill_bearing || proof == CompactionProof::FillWatermark)
            }
            None => false,
        }
    }

    fn occupies_quote_slot(&self) -> bool {
        self.terminal.is_none() || self.is_unresolved()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecoveredAuthoritativeOccurrence {
    source: PmJournalFillSourceV1,
    occurrence: PmJournalFillOccurrenceV1,
}

pub fn recover_pm_mutation_journal(
    path: impl AsRef<Path>,
    expected_scope: &PmJournalScopeV1,
) -> Result<PmJournalRecovery, PmJournalRecoveryError> {
    recover_with_lease_path(path.as_ref(), expected_scope)
}

pub(super) fn recover_with_lease_path(
    path: &Path,
    expected_scope: &PmJournalScopeV1,
) -> Result<PmJournalRecovery, PmJournalRecoveryError> {
    expected_scope.validate()?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PmJournalRecovery::empty(expected_scope.clone()));
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PmJournalRecoveryError::InvalidFileType);
    }
    if metadata.len() > MAX_PM_JOURNAL_BYTES {
        return Err(PmJournalRecoveryError::FileTooLarge {
            bytes: metadata.len(),
        });
    }
    if metadata.len() == 0 {
        return Ok(PmJournalRecovery::empty(expected_scope.clone()));
    }

    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    recover_lines(&mut reader, expected_scope)
}

fn recover_lines(
    reader: &mut impl BufRead,
    expected_scope: &PmJournalScopeV1,
) -> Result<PmJournalRecovery, PmJournalRecoveryError> {
    let mut recovery = PmJournalRecovery::empty(expected_scope.clone());
    let mut line = Vec::with_capacity(1_024);
    let mut expected_sequence = 0_u64;
    let mut bytes_read = 0_u64;

    while let Some(complete) = read_bounded_line(reader, &mut line)? {
        let line_bytes =
            u64::try_from(line.len()).map_err(|_| PmJournalRecoveryError::LineTooLarge)?;
        bytes_read = bytes_read
            .checked_add(line_bytes + u64::from(complete))
            .ok_or(PmJournalRecoveryError::FileTooLarge { bytes: u64::MAX })?;
        if bytes_read > MAX_PM_JOURNAL_BYTES {
            return Err(PmJournalRecoveryError::FileTooLarge { bytes: bytes_read });
        }
        if !complete {
            return Err(PmJournalRecoveryError::TruncatedTail);
        }
        if line.is_empty() {
            return Err(PmJournalRecoveryError::EmptyLine);
        }
        if recovery.record_count == MAX_PM_JOURNAL_RECORDS {
            return Err(PmJournalRecoveryError::TooManyRecords);
        }
        let first = line
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace());
        if first != Some(b'[') {
            return Err(PmJournalRecoveryError::WrongEnvelopeShape);
        }
        let decoded: PmJournalLineV1 = serde_json::from_slice(&line)?;
        if decoded.scope() != expected_scope.fingerprint() {
            return Err(PmJournalRecoveryError::ScopeMismatch);
        }
        if decoded.sequence() != expected_sequence {
            return Err(PmJournalRecoveryError::NonContiguousSequence {
                expected: expected_sequence,
                actual: decoded.sequence(),
            });
        }
        apply_record(&mut recovery, decoded.record(), expected_sequence)?;
        recovery.record_count += 1;
        recovery.last_sequence = expected_sequence;
        expected_sequence = next_sequence(expected_sequence)?;
        line.clear();
    }

    if recovery.record_count == 0 {
        return Err(PmJournalRecoveryError::MissingHeader);
    }
    finalize_incomplete_effects(&mut recovery);
    Ok(recovery)
}

/// Reads one line without ever growing `line` beyond the declared maximum.
///
/// `Some(true)` is a newline-terminated line, `Some(false)` is a truncated
/// final fragment, and `None` is a clean EOF between records.
fn read_bounded_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
) -> Result<Option<bool>, PmJournalRecoveryError> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(false))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let payload = newline.map_or(available, |index| &available[..index]);
        if line.len().saturating_add(payload.len()) > MAX_PM_JOURNAL_LINE_BYTES {
            return Err(PmJournalRecoveryError::LineTooLarge);
        }
        line.extend_from_slice(payload);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(true));
        }
    }
}

fn apply_record(
    recovery: &mut PmJournalRecovery,
    record: &PmJournalRecordV1,
    sequence: u64,
) -> Result<(), PmJournalRecoveryError> {
    record.validate(&recovery.scope)?;
    if sequence == 0 {
        let PmJournalRecordV1::Header(header) = record else {
            return Err(PmJournalRecoveryError::MissingHeader);
        };
        if header.scope() != &recovery.scope {
            return Err(PmJournalRecoveryError::ScopeMismatch);
        }
        return Ok(());
    }
    if matches!(record, PmJournalRecordV1::Header(_)) {
        return Err(PmJournalRecoveryError::DuplicateHeader);
    }

    match record {
        PmJournalRecordV1::Header(_) => unreachable!("header rejected above"),
        PmJournalRecordV1::QuoteIntent(intent) => apply_quote_intent(recovery, *intent)?,
        PmJournalRecordV1::PlaceResult(result) => {
            apply_place_result(recovery, result)?;
        }
        PmJournalRecordV1::CancelIntent(intent) => {
            let order = checked_bound_order(
                &mut recovery.owned_orders,
                intent.client_order,
                intent.venue_order,
            )?;
            if order.terminal.is_some() || order.cancel_pending || order.cancel_unknown {
                return Err(PmJournalRecoveryError::InvalidCancelTransition);
            }
            order.cancel_pending = true;
        }
        PmJournalRecordV1::CancelResult(result) => {
            apply_cancel_result(recovery, result)?;
        }
        PmJournalRecordV1::FillApplied(applied) => {
            apply_fill(recovery, *applied)?;
        }
        PmJournalRecordV1::OrderTerminal(terminal) => {
            apply_terminal(recovery, terminal)?;
        }
        PmJournalRecordV1::SafetyHalt(halt) => {
            recovery.safety_halt = Some(halt.reason);
        }
        PmJournalRecordV1::FillWatermarkAdvanced(watermark) => {
            if recovery.fill_watermark == Some(watermark.cursor) {
                return Err(PmJournalRecoveryError::DuplicateWatermark);
            }
            recovery.fill_watermark = Some(watermark.cursor);
            compact_proven_terminal_orders(recovery, CompactionProof::FillWatermark);
        }
    }
    Ok(())
}

fn apply_quote_intent(
    recovery: &mut PmJournalRecovery,
    intent: PmJournalQuoteIntentV1,
) -> Result<(), PmJournalRecoveryError> {
    if intent.intent_id <= recovery.last_intent_id {
        return Err(PmJournalRecoveryError::NonMonotonicIntentId {
            prior: recovery.last_intent_id,
            actual: intent.intent_id,
        });
    }

    // A later same-slot intent is itself the documented proof that a
    // previously accepted cancellation converged and released that slot.
    let retired = recovery
        .owned_orders
        .iter()
        .filter_map(|(client, order)| {
            (same_slot(order.intent, intent)
                && matches!(
                    order.terminal,
                    Some(
                        PmJournalTerminalStatusV1::Cancelled
                            | PmJournalTerminalStatusV1::Expired
                            | PmJournalTerminalStatusV1::Rejected
                    )
                )
                && order.can_compact(CompactionProof::SlotReplacementOrTerminal))
            .then_some(*client)
        })
        .collect::<Vec<_>>();
    for client in retired {
        compact_order(recovery, client);
    }
    compact_proven_terminal_orders(recovery, CompactionProof::Automatic);

    if recovery.owned_orders.contains_key(&intent.client_order) {
        return Err(PmJournalRecoveryError::DuplicateQuoteIntent);
    }
    if recovery
        .owned_orders
        .values()
        .any(|order| same_slot(order.intent, intent) && order.occupies_quote_slot())
    {
        return Err(PmJournalRecoveryError::QuoteSlotOccupied);
    }
    if recovery.owned_orders.len() == MAX_PM_JOURNAL_OWNED_ORDERS {
        return Err(PmJournalRecoveryError::TooManyOwnedOrders);
    }
    recovery.owned_orders.insert(
        intent.client_order,
        RecoveredOwnedOrder {
            intent,
            known_fill_total: U256::ZERO,
            authoritative_cumulative: None,
            cumulative: U256::ZERO,
            last_authoritative_occurrence: None,
            venue_order: None,
            place: RecoveredPlace::IntentOnly,
            cancel_pending: false,
            cancel_unknown: false,
            pending_ack_fills: PendingAcknowledgementFills::empty(),
            terminal: None,
        },
    );
    recovery
        .order_order
        .insert(intent.intent_id, intent.client_order);
    recovery.last_intent_id = intent.intent_id;
    Ok(())
}

fn same_slot(first: PmJournalQuoteIntentV1, second: PmJournalQuoteIntentV1) -> bool {
    first.client_order.account() == second.client_order.account()
        && first.instrument == second.instrument
        && first.side == second.side
}

fn apply_place_result(
    recovery: &mut PmJournalRecovery,
    result: &super::schema::PmJournalPlaceResultV1,
) -> Result<(), PmJournalRecoveryError> {
    if let Some(venue_order) = result.venue_order
        && recovery.owned_orders.iter().any(|(client, order)| {
            *client != result.client_order && order.venue_order == Some(venue_order)
        })
    {
        return Err(PmJournalRecoveryError::DuplicateVenueBinding);
    }
    let order = recovery
        .owned_orders
        .get_mut(&result.client_order)
        .ok_or(PmJournalRecoveryError::UnknownOwnedOrder)?;
    match result.outcome {
        PmJournalPlaceOutcomeV1::AmbiguousTimeout => {
            if order.place != RecoveredPlace::IntentOnly {
                return Err(PmJournalRecoveryError::InvalidPlaceTransition);
            }
            order.place = RecoveredPlace::Unknown;
        }
        PmJournalPlaceOutcomeV1::Rejected => {
            if order.place != RecoveredPlace::IntentOnly {
                return Err(PmJournalRecoveryError::InvalidPlaceTransition);
            }
            order.place = RecoveredPlace::Rejected;
            order.terminal = Some(PmJournalTerminalStatusV1::Rejected);
        }
        PmJournalPlaceOutcomeV1::AcceptedResting
        | PmJournalPlaceOutcomeV1::AcceptedWithImmediateFill => {
            if order.place != RecoveredPlace::IntentOnly {
                return Err(PmJournalRecoveryError::InvalidPlaceTransition);
            }
            bind_venue(order, result.venue_order, &result.immediate_fills)?;
        }
        PmJournalPlaceOutcomeV1::LateAcknowledgement => {
            if order.place != RecoveredPlace::Unknown {
                return Err(PmJournalRecoveryError::InvalidPlaceTransition);
            }
            bind_venue(order, result.venue_order, &result.immediate_fills)?;
        }
    }
    compact_proven_terminal_orders(recovery, CompactionProof::Automatic);
    Ok(())
}

fn bind_venue(
    order: &mut RecoveredOwnedOrder,
    venue_order: Option<PmVenueOrderKey>,
    immediate_fills: &PmJournalImmediateFillsV1,
) -> Result<(), PmJournalRecoveryError> {
    let venue_order = venue_order.ok_or(PmJournalRecoveryError::InvalidPlaceTransition)?;
    order.venue_order = Some(venue_order);
    order.place = RecoveredPlace::Bound;
    order.pending_ack_fills.replace_from(immediate_fills);
    Ok(())
}

fn apply_cancel_result(
    recovery: &mut PmJournalRecovery,
    result: &super::schema::PmJournalCancelResultV1,
) -> Result<(), PmJournalRecoveryError> {
    let order = checked_bound_order(
        &mut recovery.owned_orders,
        result.client_order,
        result.venue_order,
    )?;
    if !order.cancel_pending {
        return Err(PmJournalRecoveryError::InvalidCancelTransition);
    }
    match result.outcome {
        PmJournalCancelOutcomeV1::Accepted => {
            order.cancel_pending = false;
            order.cancel_unknown = false;
            order.terminal = Some(if order.cumulative == order.original() {
                PmJournalTerminalStatusV1::Filled
            } else {
                PmJournalTerminalStatusV1::Cancelled
            });
        }
        PmJournalCancelOutcomeV1::Rejected => {
            order.cancel_pending = false;
            order.cancel_unknown = false;
        }
        PmJournalCancelOutcomeV1::AlreadyFilled => {
            order.cancel_pending = false;
            if order.cumulative == order.original() {
                order.cancel_unknown = false;
                order.terminal = Some(PmJournalTerminalStatusV1::Filled);
            } else {
                // The result proves the remote order is terminal, but missing
                // fills must arrive or complete reconciliation must resolve
                // exact progress before reservation release.
                order.cancel_unknown = true;
            }
        }
        PmJournalCancelOutcomeV1::AmbiguousTimeout => {
            order.cancel_unknown = true;
        }
    }
    compact_proven_terminal_orders(recovery, CompactionProof::Automatic);
    Ok(())
}

fn apply_fill(
    recovery: &mut PmJournalRecovery,
    applied: PmJournalFillAppliedV1,
) -> Result<(), PmJournalRecoveryError> {
    let fill = applied.fill;
    if recovery.fill_keys.contains_key(&fill.key) {
        return Err(PmJournalRecoveryError::DuplicateFill);
    }
    let owner_sequence = applied.occurrence.owner_sequence.value();
    if owner_sequence <= recovery.last_owned_observation_sequence {
        return Err(
            PmJournalRecoveryError::NonMonotonicOwnedObservationSequence {
                prior: recovery.last_owned_observation_sequence,
                actual: owner_sequence,
            },
        );
    }
    if recovery.fill_keys.len() == MAX_PM_JOURNAL_FILL_KEYS {
        return Err(PmJournalRecoveryError::TooManyFillKeys);
    }
    let order = recovery
        .owned_orders
        .get_mut(&fill.client_order)
        .ok_or(PmJournalRecoveryError::UnknownOwnedOrder)?;
    if order.place != RecoveredPlace::Bound
        || order.venue_order != Some(fill.key.venue_order)
        || order.intent.instrument != fill.instrument
        || order.intent.side != fill.side
        || order.terminal == Some(PmJournalTerminalStatusV1::Rejected)
        || (applied.source == PmJournalFillSourceV1::PlaceAcknowledgement
            && !order.pending_ack_fills.contains(fill.key))
    {
        return Err(PmJournalRecoveryError::InvalidFillOwnership);
    }

    let original = order.original();
    let known_fill_total = order
        .known_fill_total
        .checked_add(fill.delta.protocol_units())
        .map_err(|_| PmJournalRecoveryError::InvalidProgress)?;
    if known_fill_total > original {
        return Err(PmJournalRecoveryError::InvalidProgress);
    }

    let mut authoritative = order.authoritative_cumulative;
    if let Some(reported) = fill.authoritative_cumulative {
        if reported > original {
            return Err(PmJournalRecoveryError::InvalidProgress);
        }
        let occurrence = RecoveredAuthoritativeOccurrence {
            source: applied.source,
            occurrence: applied.occurrence,
        };
        match (authoritative, order.last_authoritative_occurrence) {
            (Some(prior), Some(prior_occurrence)) => {
                match compare_occurrences(occurrence, prior_occurrence) {
                    Some(Ordering::Less) => {
                        if reported > prior {
                            return Err(
                                PmJournalRecoveryError::ContradictoryAuthoritativeCumulative,
                            );
                        }
                        // An older observation cannot replace newer
                        // authoritative progress. Its unique fill leg still
                        // contributes independently to `known_fill_total`.
                    }
                    Some(Ordering::Equal) => {
                        if reported != prior {
                            return Err(
                                PmJournalRecoveryError::ContradictoryAuthoritativeCumulative,
                            );
                        }
                    }
                    Some(Ordering::Greater) => {
                        if reported < prior {
                            return Err(PmJournalRecoveryError::BackwardsAuthoritativeCumulative);
                        }
                        authoritative = Some(reported);
                        order.last_authoritative_occurrence = Some(occurrence);
                    }
                    None => {
                        // Different source domains do not share a comparable
                        // venue predecessor sequence. Preserve the maximum
                        // fact without pretending coordinator delivery order
                        // establishes venue chronology.
                        if reported > prior {
                            authoritative = Some(reported);
                            order.last_authoritative_occurrence = Some(occurrence);
                        }
                    }
                }
            }
            (Some(prior), None) => {
                if reported < prior {
                    return Err(PmJournalRecoveryError::BackwardsAuthoritativeCumulative);
                }
                authoritative = Some(reported);
                order.last_authoritative_occurrence = Some(occurrence);
            }
            (None, _) => {
                authoritative = Some(reported);
                order.last_authoritative_occurrence = Some(occurrence);
            }
        }
    }
    let next_cumulative = max_units(known_fill_total, authoritative.unwrap_or(U256::ZERO));
    if fill.cumulative != next_cumulative
        || fill
            .cumulative
            .checked_add(fill.remaining)
            .map_err(|_| PmJournalRecoveryError::InvalidProgress)?
            != original
    {
        return Err(PmJournalRecoveryError::InvalidProgress);
    }

    order.known_fill_total = known_fill_total;
    order.authoritative_cumulative = authoritative;
    order.cumulative = next_cumulative;
    order.pending_ack_fills.resolve(fill.key);
    if next_cumulative == original {
        order.terminal = Some(PmJournalTerminalStatusV1::Filled);
        order.cancel_pending = false;
        order.cancel_unknown = false;
    }
    recovery.fill_keys.insert(fill.key, applied);
    recovery.fill_order.insert(owner_sequence, fill.key);
    recovery.observation_order.insert(
        owner_sequence,
        PmJournalRecoveredObservationV1::FillApplied(applied),
    );
    recovery.last_owned_observation_sequence = owner_sequence;
    compact_proven_terminal_orders(recovery, CompactionProof::Automatic);
    Ok(())
}

fn compare_occurrences(
    next: RecoveredAuthoritativeOccurrence,
    prior: RecoveredAuthoritativeOccurrence,
) -> Option<Ordering> {
    if next.source == prior.source && next.occurrence.connection == prior.occurrence.connection {
        Some(match next.source {
            PmJournalFillSourceV1::PrivateWebsocket => (
                next.occurrence
                    .connection_epoch
                    .map_or(0, |epoch| epoch.value()),
                next.occurrence
                    .ingress_sequence
                    .map_or(0, |sequence| sequence.value()),
                next.occurrence.owner_sequence.value(),
                next.occurrence.monotonic_service_ns,
            )
                .cmp(&(
                    prior
                        .occurrence
                        .connection_epoch
                        .map_or(0, |epoch| epoch.value()),
                    prior
                        .occurrence
                        .ingress_sequence
                        .map_or(0, |sequence| sequence.value()),
                    prior.occurrence.owner_sequence.value(),
                    prior.occurrence.monotonic_service_ns,
                )),
            PmJournalFillSourceV1::RestReconciliation => (
                next.occurrence
                    .snapshot_revision
                    .map_or(0, |revision| revision.value()),
                next.occurrence
                    .connection_epoch
                    .map_or(0, |epoch| epoch.value()),
                next.occurrence
                    .ingress_sequence
                    .map_or(0, |sequence| sequence.value()),
                next.occurrence.owner_sequence.value(),
                next.occurrence.monotonic_service_ns,
            )
                .cmp(&(
                    prior
                        .occurrence
                        .snapshot_revision
                        .map_or(0, |revision| revision.value()),
                    prior
                        .occurrence
                        .connection_epoch
                        .map_or(0, |epoch| epoch.value()),
                    prior
                        .occurrence
                        .ingress_sequence
                        .map_or(0, |sequence| sequence.value()),
                    prior.occurrence.owner_sequence.value(),
                    prior.occurrence.monotonic_service_ns,
                )),
            PmJournalFillSourceV1::PlaceAcknowledgement => (
                next.occurrence.owner_sequence.value(),
                next.occurrence.monotonic_service_ns,
            )
                .cmp(&(
                    prior.occurrence.owner_sequence.value(),
                    prior.occurrence.monotonic_service_ns,
                )),
        })
    } else {
        None
    }
}

fn max_units(first: U256, second: U256) -> U256 {
    if first >= second { first } else { second }
}

fn apply_terminal(
    recovery: &mut PmJournalRecovery,
    terminal: &super::schema::PmJournalOrderTerminalV1,
) -> Result<(), PmJournalRecoveryError> {
    let owner_sequence = terminal.occurrence.owner_sequence.value();
    if owner_sequence <= recovery.last_owned_observation_sequence {
        return Err(
            PmJournalRecoveryError::NonMonotonicOwnedObservationSequence {
                prior: recovery.last_owned_observation_sequence,
                actual: owner_sequence,
            },
        );
    }
    let order = recovery
        .owned_orders
        .get_mut(&terminal.client_order)
        .ok_or(PmJournalRecoveryError::UnknownOwnedOrder)?;
    let valid_status = match terminal.status {
        PmJournalTerminalStatusV1::Filled => terminal.cumulative == order.original(),
        PmJournalTerminalStatusV1::Cancelled | PmJournalTerminalStatusV1::Expired => true,
        PmJournalTerminalStatusV1::Rejected => terminal.cumulative.is_zero(),
    };
    if !valid_status
        || order.place != RecoveredPlace::Bound
        || Some(terminal.venue_order) != order.venue_order
        || terminal.cumulative != order.cumulative
        || terminal
            .cumulative
            .checked_add(terminal.remaining)
            .map_err(|_| PmJournalRecoveryError::InvalidProgress)?
            != order.original()
        || !valid_terminal_transition(order.terminal, terminal.status)
    {
        return Err(PmJournalRecoveryError::InvalidTerminalTransition);
    }
    order.terminal = Some(terminal.status);
    order.cancel_pending = false;
    order.cancel_unknown = false;
    recovery.observation_order.insert(
        owner_sequence,
        PmJournalRecoveredObservationV1::OrderTerminal(*terminal),
    );
    recovery.last_owned_observation_sequence = owner_sequence;
    compact_order_if_safe(recovery, terminal.client_order, CompactionProof::Automatic);
    Ok(())
}

const fn valid_terminal_transition(
    prior: Option<PmJournalTerminalStatusV1>,
    incoming: PmJournalTerminalStatusV1,
) -> bool {
    matches!(
        (prior, incoming),
        (None, _)
            | (
                Some(PmJournalTerminalStatusV1::Filled),
                PmJournalTerminalStatusV1::Filled
            )
            | (
                Some(PmJournalTerminalStatusV1::Rejected),
                PmJournalTerminalStatusV1::Rejected
            )
            | (
                Some(PmJournalTerminalStatusV1::Expired),
                PmJournalTerminalStatusV1::Expired
            )
            | (
                Some(PmJournalTerminalStatusV1::Cancelled),
                PmJournalTerminalStatusV1::Cancelled
            )
            | (
                Some(PmJournalTerminalStatusV1::Cancelled),
                PmJournalTerminalStatusV1::Filled
            )
    )
}

fn checked_bound_order(
    orders: &mut BTreeMap<PmClientOrderKey, RecoveredOwnedOrder>,
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
) -> Result<&mut RecoveredOwnedOrder, PmJournalRecoveryError> {
    let order = orders
        .get_mut(&client_order)
        .ok_or(PmJournalRecoveryError::UnknownOwnedOrder)?;
    if order.place != RecoveredPlace::Bound || order.venue_order != Some(venue_order) {
        return Err(PmJournalRecoveryError::UnprovenVenueOwnership);
    }
    Ok(order)
}

fn compact_proven_terminal_orders(recovery: &mut PmJournalRecovery, proof: CompactionProof) {
    let clients = recovery
        .owned_orders
        .iter()
        .filter_map(|(client, order)| order.can_compact(proof).then_some(*client))
        .collect::<Vec<_>>();
    for client in clients {
        compact_order(recovery, client);
    }
}

fn compact_order_if_safe(
    recovery: &mut PmJournalRecovery,
    client: PmClientOrderKey,
    proof: CompactionProof,
) {
    if recovery
        .owned_orders
        .get(&client)
        .is_some_and(|order| order.can_compact(proof))
    {
        compact_order(recovery, client);
    }
}

fn compact_order(recovery: &mut PmJournalRecovery, client: PmClientOrderKey) {
    let Some(removed) = recovery.owned_orders.remove(&client) else {
        return;
    };
    recovery.order_order.remove(&removed.intent.intent_id);
    recovery
        .fill_keys
        .retain(|_, applied| applied.fill.client_order != client);
    let retained_fill_keys = &recovery.fill_keys;
    recovery
        .fill_order
        .retain(|_, key| retained_fill_keys.contains_key(key));
    recovery
        .observation_order
        .retain(|_, observation| observation.client_order() != client);
    // This is a contiguous issued-intent cut, not merely the greatest
    // individually retired identity. It must never jump over an older active
    // order because downstream state may use it to reject stale bootstrap
    // facts without retaining an unbounded retired-id set.
    let safe_cut = recovery
        .owned_orders
        .values()
        .map(|order| order.intent.intent_id)
        .min()
        .map_or(recovery.last_intent_id, |oldest_active| {
            oldest_active.saturating_sub(1)
        });
    recovery.compacted_intent_id = recovery.compacted_intent_id.max(safe_cut);
}

fn finalize_incomplete_effects(recovery: &mut PmJournalRecovery) {
    for order in recovery.owned_orders.values_mut() {
        if order.place == RecoveredPlace::IntentOnly {
            order.place = RecoveredPlace::Unknown;
        }
        if order.cancel_pending {
            order.cancel_unknown = true;
        }
    }
}

#[derive(Debug, Error)]
pub enum PmJournalRecoveryError {
    #[error("PM journal IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("PM journal JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("PM journal schema failed: {0}")]
    Schema(#[from] super::schema::PmJournalSchemaError),
    #[error("PM journal path is not a regular non-symlink file")]
    InvalidFileType,
    #[error("PM journal has {bytes} bytes, above its bounded file limit")]
    FileTooLarge { bytes: u64 },
    #[error("PM journal line exceeds its bounded size")]
    LineTooLarge,
    #[error("PM journal has an incomplete final record")]
    TruncatedTail,
    #[error("PM journal contains an empty record")]
    EmptyLine,
    #[error("PM journal envelope is not the PM tuple schema")]
    WrongEnvelopeShape,
    #[error("PM journal contains more than its bounded record limit")]
    TooManyRecords,
    #[error("PM journal is missing its sequence-zero header")]
    MissingHeader,
    #[error("PM journal contains a header after sequence zero")]
    DuplicateHeader,
    #[error("PM journal line scope differs from the expected lease scope")]
    ScopeMismatch,
    #[error("PM journal sequence is not contiguous: expected {expected}, got {actual}")]
    NonContiguousSequence { expected: u64, actual: u64 },
    #[error("PM journal intent identity is not monotonic: prior {prior}, got {actual}")]
    NonMonotonicIntentId { prior: u64, actual: u64 },
    #[error("PM journal owned-observation sequence is not monotonic: prior {prior}, got {actual}")]
    NonMonotonicOwnedObservationSequence { prior: u64, actual: u64 },
    #[error("PM journal contains more than its bounded active owned-order limit")]
    TooManyOwnedOrders,
    #[error("PM journal contains more than its bounded active fill-key limit")]
    TooManyFillKeys,
    #[error("PM journal repeats a quote intent")]
    DuplicateQuoteIntent,
    #[error("PM journal quote slot is still occupied")]
    QuoteSlotOccupied,
    #[error("PM journal references an unknown locally owned order")]
    UnknownOwnedOrder,
    #[error("PM journal place result violates the owned-order transition")]
    InvalidPlaceTransition,
    #[error("PM journal cancel record violates the owned-order transition")]
    InvalidCancelTransition,
    #[error("PM journal venue identity lacks exact local ownership")]
    UnprovenVenueOwnership,
    #[error("PM journal binds one venue identity to multiple local orders")]
    DuplicateVenueBinding,
    #[error("PM journal fill lacks exact local order/side/venue ownership")]
    InvalidFillOwnership,
    #[error("PM journal repeats a fill key")]
    DuplicateFill,
    #[error("PM journal cumulative or remaining progress is inconsistent")]
    InvalidProgress,
    #[error("PM journal authoritative cumulative moved backwards in source order")]
    BackwardsAuthoritativeCumulative,
    #[error("PM journal older or identical source occurrence contradicts authoritative progress")]
    ContradictoryAuthoritativeCumulative,
    #[error("PM journal terminal record contradicts canonical order progress")]
    InvalidTerminalTransition,
    #[error("PM journal repeats a fill-eviction watermark")]
    DuplicateWatermark,
}

#[cfg(test)]
#[path = "recovery/tests.rs"]
mod tests;
