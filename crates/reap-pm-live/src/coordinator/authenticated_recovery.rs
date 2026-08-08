//! Exact cross-journal restart gate for authenticated PM mutation attempts.
//!
//! This module joins only non-secret recovery projections. It never recreates
//! a send token. A classified authenticated result missing its atomic Goal-F
//! bridge is the sole repairable case; every other mismatch fails closed.

#[cfg(any(test, feature = "loopback-evidence"))]
use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

#[cfg(any(test, feature = "loopback-evidence"))]
use crate::authenticated_journal::{
    PmAuthenticatedJournalRecovery, PmAuthenticatedPreparedWithoutGrantV1,
    PmAuthenticatedRecoveredResultClassificationV1, PmAuthenticatedRecoveredResultV1,
    PmAuthenticatedUnresolvedOperationKindV1, PmAuthenticatedUnresolvedOperationV1,
};
#[cfg(any(test, feature = "loopback-evidence"))]
use crate::journal::{
    PmJournalAuthenticatedCancelResultV1, PmJournalAuthenticatedClassificationV1,
    PmJournalAuthenticatedPlaceResultV1, PmJournalAuthenticatedResultV1, PmJournalRecovery,
};

#[cfg(any(test, feature = "loopback-evidence"))]
#[derive(Debug)]
pub(super) struct PmAuthenticatedRecoveryGate {
    missing_bridges: Box<[PmAuthenticatedRecoveredResultV1]>,
    requires_reconciliation: bool,
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl PmAuthenticatedRecoveryGate {
    pub(super) fn validate(
        goal_f: &PmJournalRecovery,
        authenticated: &PmAuthenticatedJournalRecovery,
    ) -> Result<Self, PmAuthenticatedRecoveryError> {
        if goal_f.scope().account_scope() != authenticated.scope().account_scope()
            || goal_f.scope().instrument() != authenticated.scope().instrument()
            || goal_f.scope().configuration_fingerprint().bytes()
                != authenticated.scope().configuration_fingerprint()
        {
            return Err(PmAuthenticatedRecoveryError::ScopeMismatch);
        }
        let authenticated_results = authenticated
            .classified_results()
            .iter()
            .map(|result| (result.result_journal_sequence(), *result))
            .collect::<BTreeMap<_, _>>();
        let mut bridged_sequences = BTreeSet::new();
        for bridge in goal_f.authenticated_results() {
            let sequence = bridge.auth_result_sequence();
            let recovered = authenticated_results
                .get(&sequence)
                .ok_or(PmAuthenticatedRecoveryError::ExtraGoalFBridge)?;
            if !bridge_matches_recovery(bridge, *recovered) {
                return Err(PmAuthenticatedRecoveryError::BridgeMismatch);
            }
            if !bridged_sequences.insert(sequence) {
                return Err(PmAuthenticatedRecoveryError::DuplicateGoalFBridge);
            }
        }
        validate_all_attempt_priors(goal_f, authenticated, &bridged_sequences)?;
        let missing_bridges = authenticated_results
            .into_iter()
            .filter_map(|(sequence, result)| {
                (!bridged_sequences.contains(&sequence)).then_some(result)
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            missing_bridges,
            requires_reconciliation: authenticated.requires_reconciliation()
                || !authenticated.prepared_without_grant().is_empty(),
        })
    }

    pub(super) fn missing_bridges(&self) -> &[PmAuthenticatedRecoveredResultV1] {
        &self.missing_bridges
    }

    pub(super) const fn requires_reconciliation(&self) -> bool {
        self.requires_reconciliation
    }

    pub(super) fn missing_bridge_records(
        &self,
    ) -> Result<Vec<PmJournalAuthenticatedResultV1>, PmAuthenticatedRecoveryError> {
        self.missing_bridges
            .iter()
            .map(bridge_from_recovered)
            .collect()
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn bridge_from_recovered(
    recovered: &PmAuthenticatedRecoveredResultV1,
) -> Result<PmJournalAuthenticatedResultV1, PmAuthenticatedRecoveryError> {
    let classification = recovered_classification(recovered.classification());
    match recovered.kind() {
        PmAuthenticatedUnresolvedOperationKindV1::Place => {
            let expected = recovered
                .expected_place_order_id()
                .ok_or(PmAuthenticatedRecoveryError::InvalidRecoveredResultShape)?;
            let accepted_venue_order = (classification
                == PmJournalAuthenticatedClassificationV1::Accepted)
                .then(|| exact_venue_order(recovered.client_order(), expected))
                .transpose()?;
            Ok(PmJournalAuthenticatedResultV1::Place(
                PmJournalAuthenticatedPlaceResultV1::new(
                    recovered.prepared_journal_sequence(),
                    recovered.grant_journal_sequence(),
                    recovered.result_journal_sequence(),
                    recovered.prior_goal_f_sequence(),
                    recovered.client_order(),
                    recovered.instrument(),
                    recovered.request_commitment(),
                    expected,
                    recovered.observed_place_order_id(),
                    accepted_venue_order,
                    classification,
                )
                .map_err(|_| PmAuthenticatedRecoveryError::InvalidRecoveredResultShape)?,
            ))
        }
        PmAuthenticatedUnresolvedOperationKindV1::Cancel => {
            let venue_order = recovered
                .exact_cancel_venue_order()
                .ok_or(PmAuthenticatedRecoveryError::InvalidRecoveredResultShape)?;
            let fixed = recovered
                .exact_cancel_order_id()
                .ok_or(PmAuthenticatedRecoveryError::InvalidRecoveredResultShape)?;
            Ok(PmJournalAuthenticatedResultV1::Cancel(
                PmJournalAuthenticatedCancelResultV1::new(
                    recovered.prepared_journal_sequence(),
                    recovered.grant_journal_sequence(),
                    recovered.result_journal_sequence(),
                    recovered.prior_goal_f_sequence(),
                    recovered.client_order(),
                    recovered.instrument(),
                    venue_order,
                    recovered.request_commitment(),
                    fixed,
                    recovered.observed_cancel_order_id(),
                    classification,
                )
                .map_err(|_| PmAuthenticatedRecoveryError::InvalidRecoveredResultShape)?,
            ))
        }
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn exact_venue_order(
    client_order: reap_pm_core::PmClientOrderKey,
    bytes: [u8; 32],
) -> Result<reap_pm_core::PmVenueOrderKey, PmAuthenticatedRecoveryError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut identity = String::with_capacity(66);
    identity.push_str("0x");
    for byte in bytes {
        identity.push(char::from(HEX[usize::from(byte >> 4)]));
        identity.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    let identity = reap_pm_core::PmVenueOrderId::new(&identity)
        .map_err(|_| PmAuthenticatedRecoveryError::InvalidRecoveredResultShape)?;
    Ok(reap_pm_core::PmVenueOrderKey::new(
        client_order.account(),
        identity,
    ))
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_all_attempt_priors(
    goal_f: &PmJournalRecovery,
    authenticated: &PmAuthenticatedJournalRecovery,
    bridged_result_sequences: &BTreeSet<u64>,
) -> Result<(), PmAuthenticatedRecoveryError> {
    let mut attempts = BTreeSet::new();
    let mut prior_operations = BTreeSet::new();
    for attempt in authenticated.prepared_without_grant() {
        validate_prepared_prior(goal_f, attempt)?;
        if !attempts.insert(attempt.prepared_journal_sequence()) {
            return Err(PmAuthenticatedRecoveryError::DuplicateAuthenticatedAttempt);
        }
        if !prior_operations.insert(prior_operation_key(
            attempt.kind(),
            attempt.client_order(),
            attempt.prior_goal_f_sequence(),
            attempt.exact_cancel_venue_order(),
        )) {
            return Err(PmAuthenticatedRecoveryError::DuplicateGoalFPriorAttempt);
        }
    }
    for result in authenticated.classified_results() {
        if !bridged_result_sequences.contains(&result.result_journal_sequence()) {
            validate_result_prior(goal_f, result)?;
        }
        if !attempts.insert(result.prepared_journal_sequence()) {
            return Err(PmAuthenticatedRecoveryError::DuplicateAuthenticatedAttempt);
        }
        if !prior_operations.insert(prior_operation_key(
            result.kind(),
            result.client_order(),
            result.prior_goal_f_sequence(),
            result.exact_cancel_venue_order(),
        )) {
            return Err(PmAuthenticatedRecoveryError::DuplicateGoalFPriorAttempt);
        }
    }
    for unresolved in authenticated.unresolved_operations() {
        if unresolved
            .result_journal_sequence()
            .is_none_or(|sequence| !bridged_result_sequences.contains(&sequence))
        {
            validate_unresolved_prior(goal_f, unresolved)?;
        }
        if unresolved.result_journal_sequence().is_none() {
            if !attempts.insert(unresolved.prepared_journal_sequence()) {
                return Err(PmAuthenticatedRecoveryError::DuplicateAuthenticatedAttempt);
            }
            if !prior_operations.insert(prior_operation_key(
                unresolved.kind(),
                unresolved.client_order(),
                unresolved.prior_goal_f_sequence(),
                unresolved.exact_cancel_venue_order(),
            )) {
                return Err(PmAuthenticatedRecoveryError::DuplicateGoalFPriorAttempt);
            }
        }
    }
    if attempts.len() != authenticated.prepared_count() {
        return Err(PmAuthenticatedRecoveryError::DuplicateAuthenticatedAttempt);
    }
    Ok(())
}

#[cfg(any(test, feature = "loopback-evidence"))]
const fn prior_operation_key(
    kind: PmAuthenticatedUnresolvedOperationKindV1,
    client_order: reap_pm_core::PmClientOrderKey,
    prior_sequence: u64,
    venue_order: Option<reap_pm_core::PmVenueOrderKey>,
) -> (
    u8,
    reap_pm_core::PmClientOrderKey,
    u64,
    Option<reap_pm_core::PmVenueOrderKey>,
) {
    (
        match kind {
            PmAuthenticatedUnresolvedOperationKindV1::Place => 0,
            PmAuthenticatedUnresolvedOperationKindV1::Cancel => 1,
        },
        client_order,
        prior_sequence,
        venue_order,
    )
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_prepared_prior(
    goal_f: &PmJournalRecovery,
    attempt: &PmAuthenticatedPreparedWithoutGrantV1,
) -> Result<(), PmAuthenticatedRecoveryError> {
    validate_prior(
        goal_f,
        attempt.kind(),
        attempt.client_order(),
        attempt.instrument(),
        attempt.exact_cancel_venue_order(),
        attempt.prior_goal_f_sequence(),
    )
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_result_prior(
    goal_f: &PmJournalRecovery,
    result: &PmAuthenticatedRecoveredResultV1,
) -> Result<(), PmAuthenticatedRecoveryError> {
    validate_prior(
        goal_f,
        result.kind(),
        result.client_order(),
        result.instrument(),
        result.exact_cancel_venue_order(),
        result.prior_goal_f_sequence(),
    )
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_unresolved_prior(
    goal_f: &PmJournalRecovery,
    unresolved: &PmAuthenticatedUnresolvedOperationV1,
) -> Result<(), PmAuthenticatedRecoveryError> {
    validate_prior(
        goal_f,
        unresolved.kind(),
        unresolved.client_order(),
        unresolved.instrument(),
        unresolved.exact_cancel_venue_order(),
        unresolved.prior_goal_f_sequence(),
    )
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_prior(
    goal_f: &PmJournalRecovery,
    kind: PmAuthenticatedUnresolvedOperationKindV1,
    client_order: reap_pm_core::PmClientOrderKey,
    instrument: reap_pm_core::PmInstrumentId,
    venue_order: Option<reap_pm_core::PmVenueOrderKey>,
    prior_sequence: u64,
) -> Result<(), PmAuthenticatedRecoveryError> {
    let order = goal_f
        .recovered_order(client_order)
        .ok_or(PmAuthenticatedRecoveryError::MissingGoalFIntent)?;
    if order.intent().instrument != instrument {
        return Err(PmAuthenticatedRecoveryError::AttemptScopeMismatch);
    }
    if match kind {
        PmAuthenticatedUnresolvedOperationKindV1::Place => {
            goal_f.has_ordinary_place_result(client_order)
        }
        PmAuthenticatedUnresolvedOperationKindV1::Cancel => {
            goal_f.has_ordinary_cancel_result(client_order)
        }
    } {
        return Err(PmAuthenticatedRecoveryError::PriorAlreadyCompletedByOrdinaryResult);
    }
    let valid = match kind {
        PmAuthenticatedUnresolvedOperationKindV1::Place => {
            venue_order.is_none() && order.intent_journal_sequence() == prior_sequence
        }
        PmAuthenticatedUnresolvedOperationKindV1::Cancel => {
            venue_order.is_some()
                && order.venue_order() == venue_order
                && order.cancel_intent_journal_sequence() == Some(prior_sequence)
        }
    };
    if valid {
        Ok(())
    } else {
        Err(PmAuthenticatedRecoveryError::PriorGoalFSequenceMismatch)
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn bridge_matches_recovery(
    bridge: PmJournalAuthenticatedResultV1,
    recovered: PmAuthenticatedRecoveredResultV1,
) -> bool {
    match bridge {
        PmJournalAuthenticatedResultV1::Place(bridge) => {
            recovered.kind() == PmAuthenticatedUnresolvedOperationKindV1::Place
                && bridge.auth_prepared_sequence() == recovered.prepared_journal_sequence()
                && bridge.auth_grant_sequence() == recovered.grant_journal_sequence()
                && bridge.auth_result_sequence() == recovered.result_journal_sequence()
                && bridge.prior_goal_f_sequence() == recovered.prior_goal_f_sequence()
                && bridge.canonical().client_order == recovered.client_order()
                && bridge.instrument() == recovered.instrument()
                && bridge.request_commitment() == recovered.request_commitment()
                && Some(bridge.expected_order_id()) == recovered.expected_place_order_id()
                && bridge.observed_order_id() == recovered.observed_place_order_id()
                && bridge.classification() == recovered_classification(recovered.classification())
        }
        PmJournalAuthenticatedResultV1::Cancel(bridge) => {
            recovered.kind() == PmAuthenticatedUnresolvedOperationKindV1::Cancel
                && bridge.auth_prepared_sequence() == recovered.prepared_journal_sequence()
                && bridge.auth_grant_sequence() == recovered.grant_journal_sequence()
                && bridge.auth_result_sequence() == recovered.result_journal_sequence()
                && bridge.prior_goal_f_sequence() == recovered.prior_goal_f_sequence()
                && bridge.canonical().client_order == recovered.client_order()
                && bridge.canonical().venue_order == recovered.exact_cancel_venue_order().unwrap()
                && bridge.instrument() == recovered.instrument()
                && bridge.request_commitment() == recovered.request_commitment()
                && Some(bridge.fixed_order_id()) == recovered.exact_cancel_order_id()
                && bridge.observed_order_id() == recovered.observed_cancel_order_id()
                && bridge.classification() == recovered_classification(recovered.classification())
        }
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
const fn recovered_classification(
    recovered: PmAuthenticatedRecoveredResultClassificationV1,
) -> PmJournalAuthenticatedClassificationV1 {
    use crate::authenticated_journal::{
        PmAuthenticatedCancelResultKindV1 as Cancel, PmAuthenticatedPlaceResultKindV1 as Place,
    };
    match recovered {
        PmAuthenticatedRecoveredResultClassificationV1::Place(Place::Accepted)
        | PmAuthenticatedRecoveredResultClassificationV1::Cancel(Cancel::Accepted) => {
            PmJournalAuthenticatedClassificationV1::Accepted
        }
        PmAuthenticatedRecoveredResultClassificationV1::Place(Place::Rejected)
        | PmAuthenticatedRecoveredResultClassificationV1::Cancel(Cancel::Rejected) => {
            PmJournalAuthenticatedClassificationV1::Rejected
        }
        PmAuthenticatedRecoveredResultClassificationV1::Place(Place::DefinitelyNotDispatched)
        | PmAuthenticatedRecoveredResultClassificationV1::Cancel(Cancel::DefinitelyNotDispatched) => {
            PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched
        }
        PmAuthenticatedRecoveredResultClassificationV1::Place(Place::OutOfProfile)
        | PmAuthenticatedRecoveredResultClassificationV1::Cancel(Cancel::OutOfProfile) => {
            PmJournalAuthenticatedClassificationV1::OutOfProfile
        }
        PmAuthenticatedRecoveredResultClassificationV1::Place(Place::AcknowledgementUnknown)
        | PmAuthenticatedRecoveredResultClassificationV1::Cancel(Cancel::AcknowledgementUnknown) => {
            PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown
        }
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PmAuthenticatedRecoveryError {
    #[error("authenticated and Goal-F journal scopes do not match")]
    ScopeMismatch,
    #[error("authenticated attempt has no retained Goal-F intent")]
    MissingGoalFIntent,
    #[error("authenticated attempt lies outside its Goal-F intent scope")]
    AttemptScopeMismatch,
    #[error("authenticated attempt points to the wrong Goal-F intent/cancel sequence")]
    PriorGoalFSequenceMismatch,
    #[error("authenticated attempt reuses an intent completed by an ordinary Goal-F result")]
    PriorAlreadyCompletedByOrdinaryResult,
    #[error("authenticated recovery repeats or omits an attempt projection")]
    DuplicateAuthenticatedAttempt,
    #[error("multiple authenticated attempts reuse the same exact Goal-F prior authority")]
    DuplicateGoalFPriorAttempt,
    #[error("Goal-F contains an authenticated bridge absent from the auth journal")]
    ExtraGoalFBridge,
    #[error("Goal-F repeats an authenticated result bridge")]
    DuplicateGoalFBridge,
    #[error("Goal-F authenticated bridge does not exactly match the auth journal result")]
    BridgeMismatch,
    #[error("authenticated recovered result cannot form the exact Goal-F bridge")]
    InvalidRecoveredResultShape,
}

#[cfg(not(any(test, feature = "loopback-evidence")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PmAuthenticatedRecoveryError {}

#[cfg(test)]
#[path = "authenticated_recovery/tests.rs"]
mod tests;
