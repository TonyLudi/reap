use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

use reap_pm_core::{PmClientOrderKey, PmInstrumentId, PmVenueOrderKey};
use thiserror::Error;

use super::schema::{
    MAX_PM_AUTHENTICATED_JOURNAL_BYTES, MAX_PM_AUTHENTICATED_JOURNAL_LINE_BYTES,
    MAX_PM_AUTHENTICATED_JOURNAL_RECORDS, PmAuthenticatedCancelPreparedV1,
    PmAuthenticatedCancelResultKindV1, PmAuthenticatedJournalLineV1,
    PmAuthenticatedJournalRecordV1, PmAuthenticatedJournalSchemaError,
    PmAuthenticatedJournalScopeV1, PmAuthenticatedOperationKeyV1, PmAuthenticatedPlacePreparedV1,
    PmAuthenticatedPlaceResultKindV1, next_sequence,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmAuthenticatedRecoveredResultClassificationV1 {
    Place(PmAuthenticatedPlaceResultKindV1),
    Cancel(PmAuthenticatedCancelResultKindV1),
}

impl PmAuthenticatedRecoveredResultClassificationV1 {
    const fn requires_reconciliation(self) -> bool {
        matches!(
            self,
            Self::Place(PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown)
                | Self::Place(PmAuthenticatedPlaceResultKindV1::OutOfProfile)
                | Self::Cancel(PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown)
                | Self::Cancel(PmAuthenticatedCancelResultKindV1::OutOfProfile)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecoveredResult {
    classification: PmAuthenticatedRecoveredResultClassificationV1,
    journal_sequence: u64,
    observed_order_id: Option<super::schema::PmAuthenticatedCommitmentV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveredOrderIdentity {
    Place {
        expected_order_id: super::schema::PmAuthenticatedCommitmentV1,
    },
    Cancel {
        exact_order_id: super::schema::PmAuthenticatedCommitmentV1,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecoveredAttempt {
    operation: PmAuthenticatedOperationKeyV1,
    prior_sequence: u64,
    request_commitment: super::schema::PmAuthenticatedCommitmentV1,
    order_identity: RecoveredOrderIdentity,
    prepared_sequence: u64,
    grant_sequence: Option<u64>,
    result: Option<RecoveredResult>,
}

impl RecoveredAttempt {
    const fn expected_place_order_id(self) -> Option<super::schema::PmAuthenticatedCommitmentV1> {
        match self.order_identity {
            RecoveredOrderIdentity::Place { expected_order_id } => Some(expected_order_id),
            RecoveredOrderIdentity::Cancel { .. } => None,
        }
    }
}

/// Exact read-only projection of one durable classified authenticated result.
///
/// This type contains no durable acknowledgement, authenticated request bytes,
/// or authority to dispatch or append another transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmAuthenticatedRecoveredResultV1 {
    operation: PmAuthenticatedOperationKeyV1,
    prior_goal_f_sequence: u64,
    prepared_journal_sequence: u64,
    grant_journal_sequence: u64,
    result_journal_sequence: u64,
    request_commitment: [u8; 32],
    expected_or_exact_order_id: [u8; 32],
    observed_order_id: Option<[u8; 32]>,
    classification: PmAuthenticatedRecoveredResultClassificationV1,
}

impl PmAuthenticatedRecoveredResultV1 {
    #[must_use]
    pub(crate) const fn kind(&self) -> PmAuthenticatedUnresolvedOperationKindV1 {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => {
                PmAuthenticatedUnresolvedOperationKindV1::Place
            }
            PmAuthenticatedOperationKeyV1::Cancel { .. } => {
                PmAuthenticatedUnresolvedOperationKindV1::Cancel
            }
        }
    }

    #[must_use]
    pub(crate) const fn classification(&self) -> PmAuthenticatedRecoveredResultClassificationV1 {
        self.classification
    }

    #[must_use]
    pub(crate) const fn client_order(&self) -> PmClientOrderKey {
        self.operation.coordinator().client_order
    }

    #[must_use]
    pub(crate) const fn instrument(&self) -> PmInstrumentId {
        self.operation.coordinator().instrument
    }

    #[must_use]
    pub(crate) const fn exact_cancel_venue_order(&self) -> Option<PmVenueOrderKey> {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => None,
            PmAuthenticatedOperationKeyV1::Cancel { venue_order, .. } => Some(venue_order),
        }
    }

    #[must_use]
    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        self.prior_goal_f_sequence
    }

    #[must_use]
    pub(crate) const fn prepared_journal_sequence(&self) -> u64 {
        self.prepared_journal_sequence
    }

    #[must_use]
    pub(crate) const fn grant_journal_sequence(&self) -> u64 {
        self.grant_journal_sequence
    }

    #[must_use]
    pub(crate) const fn result_journal_sequence(&self) -> u64 {
        self.result_journal_sequence
    }

    #[must_use]
    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        self.request_commitment
    }

    #[must_use]
    pub(crate) const fn expected_place_order_id(&self) -> Option<[u8; 32]> {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => Some(self.expected_or_exact_order_id),
            PmAuthenticatedOperationKeyV1::Cancel { .. } => None,
        }
    }

    #[must_use]
    pub(crate) const fn exact_cancel_order_id(&self) -> Option<[u8; 32]> {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => None,
            PmAuthenticatedOperationKeyV1::Cancel { .. } => Some(self.expected_or_exact_order_id),
        }
    }

    #[must_use]
    pub(crate) const fn observed_place_order_id(&self) -> Option<[u8; 32]> {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => self.observed_order_id,
            PmAuthenticatedOperationKeyV1::Cancel { .. } => None,
        }
    }

    #[must_use]
    pub(crate) const fn observed_cancel_order_id(&self) -> Option<[u8; 32]> {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => None,
            PmAuthenticatedOperationKeyV1::Cancel { .. } => self.observed_order_id,
        }
    }
}

/// A prepared request that never crossed the durable may-have-sent grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmAuthenticatedPreparedWithoutGrantV1 {
    operation: PmAuthenticatedOperationKeyV1,
    prior_goal_f_sequence: u64,
    prepared_journal_sequence: u64,
    request_commitment: [u8; 32],
    expected_or_exact_order_id: [u8; 32],
}

impl PmAuthenticatedPreparedWithoutGrantV1 {
    #[must_use]
    pub(crate) const fn kind(&self) -> PmAuthenticatedUnresolvedOperationKindV1 {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => {
                PmAuthenticatedUnresolvedOperationKindV1::Place
            }
            PmAuthenticatedOperationKeyV1::Cancel { .. } => {
                PmAuthenticatedUnresolvedOperationKindV1::Cancel
            }
        }
    }

    #[must_use]
    pub(crate) const fn client_order(&self) -> PmClientOrderKey {
        self.operation.coordinator().client_order
    }

    #[must_use]
    pub(crate) const fn instrument(&self) -> PmInstrumentId {
        self.operation.coordinator().instrument
    }

    #[must_use]
    pub(crate) const fn exact_cancel_venue_order(&self) -> Option<PmVenueOrderKey> {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => None,
            PmAuthenticatedOperationKeyV1::Cancel { venue_order, .. } => Some(venue_order),
        }
    }

    #[must_use]
    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        self.prior_goal_f_sequence
    }

    #[must_use]
    pub(crate) const fn prepared_journal_sequence(&self) -> u64 {
        self.prepared_journal_sequence
    }

    #[must_use]
    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        self.request_commitment
    }

    #[must_use]
    pub(crate) const fn expected_place_order_id(&self) -> Option<[u8; 32]> {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => Some(self.expected_or_exact_order_id),
            PmAuthenticatedOperationKeyV1::Cancel { .. } => None,
        }
    }

    #[must_use]
    pub(crate) const fn exact_cancel_order_id(&self) -> Option<[u8; 32]> {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Place { .. } => None,
            PmAuthenticatedOperationKeyV1::Cancel { .. } => Some(self.expected_or_exact_order_id),
        }
    }

    #[must_use]
    pub(crate) const fn definitely_not_sent(&self) -> bool {
        true
    }

    /// Prepared-only recovery is deliberately operator-owned, not an implicit
    /// retry authority.
    #[must_use]
    pub(crate) const fn allows_automatic_retry(&self) -> bool {
        false
    }
}

/// The operation family of one authenticated send that may have reached PM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmAuthenticatedUnresolvedOperationKindV1 {
    Place,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmAuthenticatedUnresolvedReasonV1 {
    GrantTail,
    AcknowledgementUnknown,
    OutOfProfile,
}

/// Exact non-secret reconciliation evidence recovered from a durable grant.
///
/// This projection is deliberately not a dispatch authority. It contains no
/// durable acknowledgement, authenticated request, release token, body, or
/// constructor for another `DispatchAuthorized` record. Recovery may use it
/// only to halt placement and request exact read-only reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmAuthenticatedUnresolvedOperationV1 {
    Place {
        client_order: PmClientOrderKey,
        instrument: PmInstrumentId,
        prior_intent_sequence: u64,
        prepared_journal_sequence: u64,
        grant_journal_sequence: u64,
        result_journal_sequence: Option<u64>,
        reason: PmAuthenticatedUnresolvedReasonV1,
        request_commitment: [u8; 32],
        expected_order_id: [u8; 32],
    },
    Cancel {
        client_order: PmClientOrderKey,
        instrument: PmInstrumentId,
        venue_order: PmVenueOrderKey,
        prior_cancel_sequence: u64,
        prepared_journal_sequence: u64,
        grant_journal_sequence: u64,
        result_journal_sequence: Option<u64>,
        reason: PmAuthenticatedUnresolvedReasonV1,
        request_commitment: [u8; 32],
        exact_order_id: [u8; 32],
    },
}

impl PmAuthenticatedUnresolvedOperationV1 {
    #[must_use]
    pub(crate) const fn kind(&self) -> PmAuthenticatedUnresolvedOperationKindV1 {
        match self {
            Self::Place { .. } => PmAuthenticatedUnresolvedOperationKindV1::Place,
            Self::Cancel { .. } => PmAuthenticatedUnresolvedOperationKindV1::Cancel,
        }
    }

    #[must_use]
    pub(crate) const fn client_order(&self) -> PmClientOrderKey {
        match self {
            Self::Place { client_order, .. } | Self::Cancel { client_order, .. } => *client_order,
        }
    }

    #[must_use]
    pub(crate) const fn instrument(&self) -> PmInstrumentId {
        match self {
            Self::Place { instrument, .. } | Self::Cancel { instrument, .. } => *instrument,
        }
    }

    /// Exact prior Goal F intent/cancel journal sequence.
    #[must_use]
    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        match self {
            Self::Place {
                prior_intent_sequence,
                ..
            } => *prior_intent_sequence,
            Self::Cancel {
                prior_cancel_sequence,
                ..
            } => *prior_cancel_sequence,
        }
    }

    #[must_use]
    pub(crate) const fn prepared_journal_sequence(&self) -> u64 {
        match self {
            Self::Place {
                prepared_journal_sequence,
                ..
            }
            | Self::Cancel {
                prepared_journal_sequence,
                ..
            } => *prepared_journal_sequence,
        }
    }

    #[must_use]
    pub(crate) const fn grant_journal_sequence(&self) -> u64 {
        match self {
            Self::Place {
                grant_journal_sequence,
                ..
            }
            | Self::Cancel {
                grant_journal_sequence,
                ..
            } => *grant_journal_sequence,
        }
    }

    /// Present when a classified result still requires reconciliation; absent
    /// means the durable grant is the journal tail for this operation.
    #[must_use]
    pub(crate) const fn result_journal_sequence(&self) -> Option<u64> {
        match self {
            Self::Place {
                result_journal_sequence,
                ..
            }
            | Self::Cancel {
                result_journal_sequence,
                ..
            } => *result_journal_sequence,
        }
    }

    #[must_use]
    pub(crate) const fn reason(&self) -> PmAuthenticatedUnresolvedReasonV1 {
        match self {
            Self::Place { reason, .. } | Self::Cancel { reason, .. } => *reason,
        }
    }

    #[must_use]
    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        match self {
            Self::Place {
                request_commitment, ..
            }
            | Self::Cancel {
                request_commitment, ..
            } => *request_commitment,
        }
    }

    #[must_use]
    pub(crate) const fn expected_place_order_id(&self) -> Option<[u8; 32]> {
        match self {
            Self::Place {
                expected_order_id, ..
            } => Some(*expected_order_id),
            Self::Cancel { .. } => None,
        }
    }

    #[must_use]
    pub(crate) const fn exact_cancel_order_id(&self) -> Option<[u8; 32]> {
        match self {
            Self::Place { .. } => None,
            Self::Cancel { exact_order_id, .. } => Some(*exact_order_id),
        }
    }

    #[must_use]
    pub(crate) const fn exact_cancel_venue_order(&self) -> Option<PmVenueOrderKey> {
        match self {
            Self::Place { .. } => None,
            Self::Cancel { venue_order, .. } => Some(*venue_order),
        }
    }

    #[must_use]
    pub(crate) const fn acknowledgement_unknown(&self) -> bool {
        matches!(
            self.reason(),
            PmAuthenticatedUnresolvedReasonV1::GrantTail
                | PmAuthenticatedUnresolvedReasonV1::AcknowledgementUnknown
        )
    }

    #[must_use]
    pub(crate) const fn may_have_been_sent(&self) -> bool {
        true
    }

    /// Every item must be resolved through read-only reconciliation, never resend.
    #[must_use]
    pub(crate) const fn requires_reconciliation(&self) -> bool {
        true
    }

    #[must_use]
    pub(crate) const fn allows_automatic_resend(&self) -> bool {
        false
    }
}

/// Bounded, non-authoritative recovery projection for authenticated sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PmAuthenticatedJournalRecovery {
    scope: PmAuthenticatedJournalScopeV1,
    record_count: usize,
    last_sequence: u64,
    attempts: BTreeMap<u64, RecoveredAttempt>,
    grants: BTreeMap<u64, u64>,
    prepared_without_grant: Box<[PmAuthenticatedPreparedWithoutGrantV1]>,
    classified_results: Box<[PmAuthenticatedRecoveredResultV1]>,
    unresolved_operations: Box<[PmAuthenticatedUnresolvedOperationV1]>,
}

impl PmAuthenticatedJournalRecovery {
    fn empty(scope: PmAuthenticatedJournalScopeV1) -> Self {
        Self {
            scope,
            record_count: 0,
            last_sequence: 0,
            attempts: BTreeMap::new(),
            grants: BTreeMap::new(),
            prepared_without_grant: Box::new([]),
            classified_results: Box::new([]),
            unresolved_operations: Box::new([]),
        }
    }

    pub(crate) const fn scope(&self) -> &PmAuthenticatedJournalScopeV1 {
        &self.scope
    }

    pub(crate) const fn record_count(&self) -> usize {
        self.record_count
    }

    pub(crate) const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub(crate) fn prepared_count(&self) -> usize {
        self.attempts.len()
    }

    pub(crate) fn prepared_without_authorization_count(&self) -> usize {
        self.prepared_without_grant.len()
    }

    pub(crate) fn conclusive_result_count(&self) -> usize {
        self.attempts
            .values()
            .filter(|attempt| {
                attempt
                    .result
                    .is_some_and(|result| !result.classification.requires_reconciliation())
            })
            .count()
    }

    pub(crate) fn acknowledgement_unknown_count(&self) -> usize {
        self.unresolved_operations
            .iter()
            .filter(|operation| operation.acknowledgement_unknown())
            .count()
    }

    /// A durable grant is a may-have-sent boundary. Missing or explicitly
    /// ambiguous results always require exact read-only reconciliation.
    pub(crate) fn requires_reconciliation(&self) -> bool {
        !self.unresolved_operations.is_empty()
    }

    /// Exact unresolved operations in durable-grant journal order.
    ///
    /// Values are read-only evidence. They cannot recreate the consumed send
    /// authority and therefore cannot authorize automatic retransmission.
    pub(crate) fn unresolved_operations(&self) -> &[PmAuthenticatedUnresolvedOperationV1] {
        &self.unresolved_operations
    }

    pub(crate) fn prepared_without_grant(&self) -> &[PmAuthenticatedPreparedWithoutGrantV1] {
        &self.prepared_without_grant
    }

    pub(crate) fn classified_results(&self) -> &[PmAuthenticatedRecoveredResultV1] {
        &self.classified_results
    }

    fn rebuild_projections(&mut self) {
        let mut prepared_without_grant = self
            .attempts
            .values()
            .filter(|attempt| attempt.grant_sequence.is_none())
            .map(project_prepared_without_grant)
            .collect::<Vec<_>>();
        prepared_without_grant
            .sort_unstable_by_key(PmAuthenticatedPreparedWithoutGrantV1::prepared_journal_sequence);
        self.prepared_without_grant = prepared_without_grant.into_boxed_slice();

        let mut classified_results = self
            .attempts
            .values()
            .filter_map(project_classified_result)
            .collect::<Vec<_>>();
        classified_results
            .sort_unstable_by_key(PmAuthenticatedRecoveredResultV1::result_journal_sequence);
        self.classified_results = classified_results.into_boxed_slice();

        let mut unresolved = self
            .attempts
            .values()
            .filter_map(project_unresolved_attempt)
            .collect::<Vec<_>>();
        unresolved.sort_unstable_by_key(|operation| operation.grant_journal_sequence());
        self.unresolved_operations = unresolved.into_boxed_slice();
    }
}

fn project_prepared_without_grant(
    attempt: &RecoveredAttempt,
) -> PmAuthenticatedPreparedWithoutGrantV1 {
    let expected_or_exact_order_id = match attempt.order_identity {
        RecoveredOrderIdentity::Place { expected_order_id } => expected_order_id.bytes(),
        RecoveredOrderIdentity::Cancel { exact_order_id } => exact_order_id.bytes(),
    };
    PmAuthenticatedPreparedWithoutGrantV1 {
        operation: attempt.operation,
        prior_goal_f_sequence: attempt.prior_sequence,
        prepared_journal_sequence: attempt.prepared_sequence,
        request_commitment: attempt.request_commitment.bytes(),
        expected_or_exact_order_id,
    }
}

fn project_classified_result(
    attempt: &RecoveredAttempt,
) -> Option<PmAuthenticatedRecoveredResultV1> {
    let grant_journal_sequence = attempt.grant_sequence?;
    let result = attempt.result?;
    let expected_or_exact_order_id = match attempt.order_identity {
        RecoveredOrderIdentity::Place { expected_order_id } => expected_order_id.bytes(),
        RecoveredOrderIdentity::Cancel { exact_order_id } => exact_order_id.bytes(),
    };
    Some(PmAuthenticatedRecoveredResultV1 {
        operation: attempt.operation,
        prior_goal_f_sequence: attempt.prior_sequence,
        prepared_journal_sequence: attempt.prepared_sequence,
        grant_journal_sequence,
        result_journal_sequence: result.journal_sequence,
        request_commitment: attempt.request_commitment.bytes(),
        expected_or_exact_order_id,
        observed_order_id: result.observed_order_id.map(|identity| identity.bytes()),
        classification: result.classification,
    })
}

fn project_unresolved_attempt(
    attempt: &RecoveredAttempt,
) -> Option<PmAuthenticatedUnresolvedOperationV1> {
    let grant_journal_sequence = attempt.grant_sequence?;
    let (result_journal_sequence, reason) = match attempt.result {
        None => (None, PmAuthenticatedUnresolvedReasonV1::GrantTail),
        Some(result) => match result.classification {
            PmAuthenticatedRecoveredResultClassificationV1::Place(
                PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown,
            )
            | PmAuthenticatedRecoveredResultClassificationV1::Cancel(
                PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown,
            ) => (
                Some(result.journal_sequence),
                PmAuthenticatedUnresolvedReasonV1::AcknowledgementUnknown,
            ),
            PmAuthenticatedRecoveredResultClassificationV1::Place(
                PmAuthenticatedPlaceResultKindV1::OutOfProfile,
            )
            | PmAuthenticatedRecoveredResultClassificationV1::Cancel(
                PmAuthenticatedCancelResultKindV1::OutOfProfile,
            ) => (
                Some(result.journal_sequence),
                PmAuthenticatedUnresolvedReasonV1::OutOfProfile,
            ),
            _ => return None,
        },
    };
    let coordinator = attempt.operation.coordinator();
    let common = (
        coordinator.client_order,
        coordinator.instrument,
        attempt.prepared_sequence,
        grant_journal_sequence,
        result_journal_sequence,
        reason,
        attempt.request_commitment.bytes(),
    );
    match (attempt.operation, attempt.order_identity) {
        (
            PmAuthenticatedOperationKeyV1::Place { .. },
            RecoveredOrderIdentity::Place { expected_order_id },
        ) => Some(PmAuthenticatedUnresolvedOperationV1::Place {
            client_order: common.0,
            instrument: common.1,
            prior_intent_sequence: attempt.prior_sequence,
            prepared_journal_sequence: common.2,
            grant_journal_sequence: common.3,
            result_journal_sequence: common.4,
            reason: common.5,
            request_commitment: common.6,
            expected_order_id: expected_order_id.bytes(),
        }),
        (
            PmAuthenticatedOperationKeyV1::Cancel { venue_order, .. },
            RecoveredOrderIdentity::Cancel { exact_order_id },
        ) => Some(PmAuthenticatedUnresolvedOperationV1::Cancel {
            client_order: common.0,
            instrument: common.1,
            venue_order,
            prior_cancel_sequence: attempt.prior_sequence,
            prepared_journal_sequence: common.2,
            grant_journal_sequence: common.3,
            result_journal_sequence: common.4,
            reason: common.5,
            request_commitment: common.6,
            exact_order_id: exact_order_id.bytes(),
        }),
        _ => unreachable!("prepared operation and order identity are created together"),
    }
}

pub(crate) fn recover_pm_authenticated_journal(
    path: impl AsRef<Path>,
    expected_scope: &PmAuthenticatedJournalScopeV1,
) -> Result<PmAuthenticatedJournalRecovery, PmAuthenticatedJournalRecoveryError> {
    recover_with_lease_path(path.as_ref(), expected_scope)
}

pub(super) fn recover_with_lease_path(
    path: &Path,
    expected_scope: &PmAuthenticatedJournalScopeV1,
) -> Result<PmAuthenticatedJournalRecovery, PmAuthenticatedJournalRecoveryError> {
    expected_scope.validate()?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PmAuthenticatedJournalRecovery::empty(
                expected_scope.clone(),
            ));
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PmAuthenticatedJournalRecoveryError::InvalidFileType);
    }
    if metadata.len() > MAX_PM_AUTHENTICATED_JOURNAL_BYTES {
        return Err(PmAuthenticatedJournalRecoveryError::FileTooLarge {
            bytes: metadata.len(),
        });
    }
    if metadata.len() == 0 {
        return Ok(PmAuthenticatedJournalRecovery::empty(
            expected_scope.clone(),
        ));
    }
    let file = std::fs::File::open(path)?;
    recover_lines(&mut std::io::BufReader::new(file), expected_scope)
}

pub(super) fn recover_lines(
    reader: &mut impl BufRead,
    expected_scope: &PmAuthenticatedJournalScopeV1,
) -> Result<PmAuthenticatedJournalRecovery, PmAuthenticatedJournalRecoveryError> {
    let mut recovery = PmAuthenticatedJournalRecovery::empty(expected_scope.clone());
    let mut line = Vec::with_capacity(1_024);
    let mut expected_sequence = 0_u64;
    let mut bytes_read = 0_u64;

    while let Some(complete) = read_bounded_line(reader, &mut line)? {
        let line_bytes = u64::try_from(line.len())
            .map_err(|_| PmAuthenticatedJournalRecoveryError::LineTooLarge)?;
        bytes_read = bytes_read
            .checked_add(line_bytes + u64::from(complete))
            .ok_or(PmAuthenticatedJournalRecoveryError::FileTooLarge { bytes: u64::MAX })?;
        if bytes_read > MAX_PM_AUTHENTICATED_JOURNAL_BYTES {
            return Err(PmAuthenticatedJournalRecoveryError::FileTooLarge { bytes: bytes_read });
        }
        if !complete {
            return Err(PmAuthenticatedJournalRecoveryError::TruncatedTail);
        }
        if line.is_empty() {
            return Err(PmAuthenticatedJournalRecoveryError::EmptyLine);
        }
        if recovery.record_count == MAX_PM_AUTHENTICATED_JOURNAL_RECORDS {
            return Err(PmAuthenticatedJournalRecoveryError::TooManyRecords);
        }
        if line
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            != Some(b'[')
        {
            return Err(PmAuthenticatedJournalRecoveryError::WrongEnvelopeShape);
        }
        let decoded: PmAuthenticatedJournalLineV1 = serde_json::from_slice(&line)?;
        if decoded.schema_version() != expected_scope.schema_version() {
            return Err(PmAuthenticatedJournalRecoveryError::ScopeMismatch);
        }
        if decoded.scope() != expected_scope.fingerprint() {
            return Err(PmAuthenticatedJournalRecoveryError::ScopeMismatch);
        }
        if decoded.sequence() != expected_sequence {
            return Err(PmAuthenticatedJournalRecoveryError::NonContiguousSequence {
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
        return Err(PmAuthenticatedJournalRecoveryError::MissingHeader);
    }
    recovery.rebuild_projections();
    Ok(recovery)
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
) -> Result<Option<bool>, PmAuthenticatedJournalRecoveryError> {
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
        if line.len().saturating_add(payload.len()) > MAX_PM_AUTHENTICATED_JOURNAL_LINE_BYTES {
            return Err(PmAuthenticatedJournalRecoveryError::LineTooLarge);
        }
        line.extend_from_slice(payload);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(true));
        }
    }
}

fn apply_record(
    recovery: &mut PmAuthenticatedJournalRecovery,
    record: &PmAuthenticatedJournalRecordV1,
    sequence: u64,
) -> Result<(), PmAuthenticatedJournalRecoveryError> {
    record.validate(&recovery.scope, sequence)?;
    if sequence == 0 {
        let PmAuthenticatedJournalRecordV1::Header(header) = record else {
            return Err(PmAuthenticatedJournalRecoveryError::MissingHeader);
        };
        if header.scope != recovery.scope {
            return Err(PmAuthenticatedJournalRecoveryError::ScopeMismatch);
        }
        return Ok(());
    }
    if matches!(record, PmAuthenticatedJournalRecordV1::Header(_)) {
        return Err(PmAuthenticatedJournalRecoveryError::DuplicateHeader);
    }

    match record {
        PmAuthenticatedJournalRecordV1::Header(_) => unreachable!("header rejected above"),
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared) => {
            apply_place_prepared(recovery, sequence, *prepared)?;
        }
        PmAuthenticatedJournalRecordV1::CancelPrepared(prepared) => {
            apply_cancel_prepared(recovery, sequence, *prepared)?;
        }
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(grant) => {
            let attempt = recovery
                .attempts
                .get_mut(&grant.prepared_sequence)
                .ok_or(PmAuthenticatedJournalRecoveryError::UnknownPreparedRequest)?;
            if attempt.operation != grant.operation || attempt.grant_sequence.is_some() {
                return Err(PmAuthenticatedJournalRecoveryError::InvalidAuthorizationTransition);
            }
            attempt.grant_sequence = Some(sequence);
            recovery.grants.insert(sequence, grant.prepared_sequence);
        }
        PmAuthenticatedJournalRecordV1::PlaceResult(result) => {
            let attempt = attempt_for_result(recovery, result.grant_sequence, result.operation)?;
            let PmAuthenticatedOperationKeyV1::Place { .. } = attempt.operation else {
                return Err(PmAuthenticatedJournalRecoveryError::ResultOperationMismatch);
            };
            if result.outcome == PmAuthenticatedPlaceResultKindV1::Accepted
                && result.observed_order_id != attempt.expected_place_order_id()
            {
                return Err(PmAuthenticatedJournalRecoveryError::AcceptedOrderIdentityMismatch);
            }
            attempt.result = Some(RecoveredResult {
                classification: PmAuthenticatedRecoveredResultClassificationV1::Place(
                    result.outcome,
                ),
                journal_sequence: sequence,
                observed_order_id: result.observed_order_id,
            });
        }
        PmAuthenticatedJournalRecordV1::CancelResult(result) => {
            let attempt = attempt_for_result(recovery, result.grant_sequence, result.operation)?;
            if !matches!(
                attempt.operation,
                PmAuthenticatedOperationKeyV1::Cancel { .. }
            ) {
                return Err(PmAuthenticatedJournalRecoveryError::ResultOperationMismatch);
            }
            attempt.result = Some(RecoveredResult {
                classification: PmAuthenticatedRecoveredResultClassificationV1::Cancel(
                    result.outcome,
                ),
                journal_sequence: sequence,
                observed_order_id: result.observed_order_id,
            });
        }
    }
    Ok(())
}

fn apply_place_prepared(
    recovery: &mut PmAuthenticatedJournalRecovery,
    sequence: u64,
    prepared: PmAuthenticatedPlacePreparedV1,
) -> Result<(), PmAuthenticatedJournalRecoveryError> {
    insert_prepared(
        recovery,
        RecoveredAttempt {
            operation: prepared.operation,
            prior_sequence: prepared.prior_intent_sequence,
            request_commitment: prepared.request_commitment,
            order_identity: RecoveredOrderIdentity::Place {
                expected_order_id: prepared.expected_order_id,
            },
            prepared_sequence: sequence,
            grant_sequence: None,
            result: None,
        },
    )
}

fn apply_cancel_prepared(
    recovery: &mut PmAuthenticatedJournalRecovery,
    sequence: u64,
    prepared: PmAuthenticatedCancelPreparedV1,
) -> Result<(), PmAuthenticatedJournalRecoveryError> {
    insert_prepared(
        recovery,
        RecoveredAttempt {
            operation: prepared.operation,
            prior_sequence: prepared.prior_cancel_sequence,
            request_commitment: prepared.request_commitment,
            order_identity: RecoveredOrderIdentity::Cancel {
                exact_order_id: prepared.fixed_order_id,
            },
            prepared_sequence: sequence,
            grant_sequence: None,
            result: None,
        },
    )
}

fn insert_prepared(
    recovery: &mut PmAuthenticatedJournalRecovery,
    attempt: RecoveredAttempt,
) -> Result<(), PmAuthenticatedJournalRecoveryError> {
    if recovery.attempts.values().any(|prior| {
        prior.operation == attempt.operation && prior.prior_sequence == attempt.prior_sequence
    }) {
        return Err(PmAuthenticatedJournalRecoveryError::DuplicatePreparedRequest);
    }
    recovery.attempts.insert(attempt.prepared_sequence, attempt);
    Ok(())
}

fn attempt_for_result(
    recovery: &mut PmAuthenticatedJournalRecovery,
    grant_sequence: u64,
    operation: PmAuthenticatedOperationKeyV1,
) -> Result<&mut RecoveredAttempt, PmAuthenticatedJournalRecoveryError> {
    let prepared_sequence = *recovery
        .grants
        .get(&grant_sequence)
        .ok_or(PmAuthenticatedJournalRecoveryError::UnknownAuthorization)?;
    let attempt = recovery
        .attempts
        .get_mut(&prepared_sequence)
        .expect("grant index references a retained attempt");
    if attempt.operation != operation || attempt.grant_sequence != Some(grant_sequence) {
        return Err(PmAuthenticatedJournalRecoveryError::ResultOperationMismatch);
    }
    if attempt.result.is_some() {
        return Err(PmAuthenticatedJournalRecoveryError::DuplicateResult);
    }
    Ok(attempt)
}

#[derive(Debug, Error)]
pub(crate) enum PmAuthenticatedJournalRecoveryError {
    #[error("PM authenticated journal IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("PM authenticated journal JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("PM authenticated journal schema failed: {0}")]
    Schema(#[from] PmAuthenticatedJournalSchemaError),
    #[error("PM authenticated journal path is not a regular non-symlink file")]
    InvalidFileType,
    #[error("PM authenticated journal has {bytes} bytes, above its bounded file limit")]
    FileTooLarge { bytes: u64 },
    #[error("PM authenticated journal line exceeds its bounded size")]
    LineTooLarge,
    #[error("PM authenticated journal has an incomplete final record")]
    TruncatedTail,
    #[error("PM authenticated journal contains an empty record")]
    EmptyLine,
    #[error("PM authenticated journal envelope is not its tuple schema")]
    WrongEnvelopeShape,
    #[error("PM authenticated journal contains more than its bounded record limit")]
    TooManyRecords,
    #[error("PM authenticated journal is missing its sequence-zero header")]
    MissingHeader,
    #[error("PM authenticated journal contains a header after sequence zero")]
    DuplicateHeader,
    #[error("PM authenticated journal line scope differs from the expected lease scope")]
    ScopeMismatch,
    #[error(
        "PM authenticated journal sequence is not contiguous: expected {expected}, got {actual}"
    )]
    NonContiguousSequence { expected: u64, actual: u64 },
    #[error("PM authenticated journal repeats one prepared operation/prior sequence")]
    DuplicatePreparedRequest,
    #[error("PM authenticated journal authorization references an unknown prepared request")]
    UnknownPreparedRequest,
    #[error("PM authenticated journal authorization transition is duplicated or mismatched")]
    InvalidAuthorizationTransition,
    #[error("PM authenticated journal result references an unknown authorization")]
    UnknownAuthorization,
    #[error("PM authenticated journal result operation differs from its durable grant")]
    ResultOperationMismatch,
    #[error("PM authenticated journal repeats a result for one durable grant")]
    DuplicateResult,
    #[error("PM authenticated journal accepted place ID differs from its signed expected ID")]
    AcceptedOrderIdentityMismatch,
}
