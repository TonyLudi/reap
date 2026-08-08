//! Atomic Goal-F bridge records for one already-durable authenticated result.
//!
//! These records contain only non-secret identities.  The authenticated
//! journal remains the send-authority ledger; this record merely makes the
//! canonical lifecycle reduction and its cross-journal identity one durable,
//! replayable Goal-F transition.

use reap_pm_core::{PmClientOrderKey, PmInstrumentId, PmVenueOrderKey};
use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "loopback-evidence"))]
use super::PmJournalImmediateFillsV1;
use super::{
    PmJournalCancelOutcomeV1, PmJournalCancelRejectReasonV1, PmJournalCancelResultV1,
    PmJournalFingerprintV1, PmJournalPlaceOutcomeV1, PmJournalPlaceRejectReasonV1,
    PmJournalPlaceResultV1, PmJournalSchemaError, PmJournalScopeV1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalAuthenticatedClassificationV1 {
    Accepted,
    Rejected,
    DefinitelyNotDispatched,
    OutOfProfile,
    AcknowledgementUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalAuthenticatedPlaceResultV1 {
    auth_prepared_sequence: u64,
    auth_grant_sequence: u64,
    auth_result_sequence: u64,
    prior_goal_f_sequence: u64,
    instrument: PmInstrumentId,
    request_commitment: PmJournalFingerprintV1,
    expected_order_id: PmJournalFingerprintV1,
    observed_order_id: Option<PmJournalFingerprintV1>,
    classification: PmJournalAuthenticatedClassificationV1,
    canonical: PmJournalPlaceResultV1,
}

impl PmJournalAuthenticatedPlaceResultV1 {
    #[cfg(any(test, feature = "loopback-evidence"))]
    #[allow(
        clippy::too_many_arguments,
        reason = "cross-journal identity is explicit"
    )]
    pub(crate) fn new(
        auth_prepared_sequence: u64,
        auth_grant_sequence: u64,
        auth_result_sequence: u64,
        prior_goal_f_sequence: u64,
        client_order: PmClientOrderKey,
        instrument: PmInstrumentId,
        request_commitment: [u8; 32],
        expected_order_id: [u8; 32],
        observed_order_id: Option<[u8; 32]>,
        accepted_venue_order: Option<PmVenueOrderKey>,
        classification: PmJournalAuthenticatedClassificationV1,
    ) -> Result<Self, PmJournalSchemaError> {
        let (outcome, reject_reason, venue_order) = match classification {
            PmJournalAuthenticatedClassificationV1::Accepted => (
                PmJournalPlaceOutcomeV1::AcceptedResting,
                None,
                accepted_venue_order,
            ),
            PmJournalAuthenticatedClassificationV1::Rejected => (
                PmJournalPlaceOutcomeV1::Rejected,
                Some(PmJournalPlaceRejectReasonV1::AuthenticatedVenueRejected),
                None,
            ),
            PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched => (
                PmJournalPlaceOutcomeV1::Rejected,
                Some(PmJournalPlaceRejectReasonV1::AuthenticatedDefinitelyNotDispatched),
                None,
            ),
            PmJournalAuthenticatedClassificationV1::OutOfProfile
            | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown => {
                (PmJournalPlaceOutcomeV1::AmbiguousTimeout, None, None)
            }
        };
        let value = Self {
            auth_prepared_sequence,
            auth_grant_sequence,
            auth_result_sequence,
            prior_goal_f_sequence,
            instrument,
            request_commitment: PmJournalFingerprintV1::from_bytes(request_commitment),
            expected_order_id: PmJournalFingerprintV1::from_bytes(expected_order_id),
            observed_order_id: observed_order_id.map(PmJournalFingerprintV1::from_bytes),
            classification,
            canonical: PmJournalPlaceResultV1 {
                client_order,
                outcome,
                reject_reason,
                venue_order,
                immediate_fills: PmJournalImmediateFillsV1::empty(),
            },
        };
        value.validate_shape()?;
        Ok(value)
    }

    pub(crate) const fn auth_prepared_sequence(&self) -> u64 {
        self.auth_prepared_sequence
    }

    pub(crate) const fn auth_grant_sequence(&self) -> u64 {
        self.auth_grant_sequence
    }

    pub(crate) const fn auth_result_sequence(&self) -> u64 {
        self.auth_result_sequence
    }

    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        self.prior_goal_f_sequence
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentId {
        self.instrument
    }

    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        self.request_commitment.bytes()
    }

    pub(crate) const fn expected_order_id(&self) -> [u8; 32] {
        self.expected_order_id.bytes()
    }

    pub(crate) const fn observed_order_id(&self) -> Option<[u8; 32]> {
        match self.observed_order_id {
            Some(identity) => Some(identity.bytes()),
            None => None,
        }
    }

    pub(crate) const fn classification(&self) -> PmJournalAuthenticatedClassificationV1 {
        self.classification
    }

    pub(crate) const fn canonical(&self) -> &PmJournalPlaceResultV1 {
        &self.canonical
    }

    pub(crate) fn validate(&self, scope: &PmJournalScopeV1) -> Result<(), PmJournalSchemaError> {
        self.canonical.validate(scope)?;
        if self.instrument != scope.instrument() {
            return Err(PmJournalSchemaError::RecordOutsideScope);
        }
        self.validate_shape()
    }

    fn validate_shape(&self) -> Result<(), PmJournalSchemaError> {
        validate_sequence_chain(
            self.auth_prepared_sequence,
            self.auth_grant_sequence,
            self.auth_result_sequence,
            self.prior_goal_f_sequence,
        )?;
        let observed_matches_expected = self.observed_order_id == Some(self.expected_order_id);
        let venue_matches_expected = self
            .canonical
            .venue_order
            .and_then(exact_order_bytes)
            .is_some_and(|bytes| bytes == self.expected_order_id.bytes());
        let valid = match self.classification {
            PmJournalAuthenticatedClassificationV1::Accepted => {
                self.canonical.outcome == PmJournalPlaceOutcomeV1::AcceptedResting
                    && self.canonical.reject_reason.is_none()
                    && observed_matches_expected
                    && venue_matches_expected
            }
            PmJournalAuthenticatedClassificationV1::Rejected => {
                rejected_place(
                    &self.canonical,
                    PmJournalPlaceRejectReasonV1::AuthenticatedVenueRejected,
                ) && self.observed_order_id.is_none()
            }
            PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched => {
                rejected_place(
                    &self.canonical,
                    PmJournalPlaceRejectReasonV1::AuthenticatedDefinitelyNotDispatched,
                ) && self.observed_order_id.is_none()
            }
            PmJournalAuthenticatedClassificationV1::OutOfProfile
            | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown => {
                self.canonical.outcome == PmJournalPlaceOutcomeV1::AmbiguousTimeout
                    && self.canonical.reject_reason.is_none()
                    && self.canonical.venue_order.is_none()
            }
        };
        if valid && self.canonical.immediate_fills.iter().next().is_none() {
            Ok(())
        } else {
            Err(PmJournalSchemaError::InvalidAuthenticatedResult)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalAuthenticatedCancelResultV1 {
    auth_prepared_sequence: u64,
    auth_grant_sequence: u64,
    auth_result_sequence: u64,
    prior_goal_f_sequence: u64,
    instrument: PmInstrumentId,
    request_commitment: PmJournalFingerprintV1,
    fixed_order_id: PmJournalFingerprintV1,
    observed_order_id: Option<PmJournalFingerprintV1>,
    classification: PmJournalAuthenticatedClassificationV1,
    canonical: PmJournalCancelResultV1,
}

impl PmJournalAuthenticatedCancelResultV1 {
    #[cfg(any(test, feature = "loopback-evidence"))]
    #[allow(
        clippy::too_many_arguments,
        reason = "cross-journal identity is explicit"
    )]
    pub(crate) fn new(
        auth_prepared_sequence: u64,
        auth_grant_sequence: u64,
        auth_result_sequence: u64,
        prior_goal_f_sequence: u64,
        client_order: PmClientOrderKey,
        instrument: PmInstrumentId,
        venue_order: PmVenueOrderKey,
        request_commitment: [u8; 32],
        fixed_order_id: [u8; 32],
        observed_order_id: Option<[u8; 32]>,
        classification: PmJournalAuthenticatedClassificationV1,
    ) -> Result<Self, PmJournalSchemaError> {
        let (outcome, reject_reason) = match classification {
            PmJournalAuthenticatedClassificationV1::Accepted => {
                (PmJournalCancelOutcomeV1::Accepted, None)
            }
            PmJournalAuthenticatedClassificationV1::Rejected => (
                PmJournalCancelOutcomeV1::Rejected,
                Some(PmJournalCancelRejectReasonV1::AuthenticatedVenueRejected),
            ),
            PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched => (
                PmJournalCancelOutcomeV1::Rejected,
                Some(PmJournalCancelRejectReasonV1::AuthenticatedDefinitelyNotDispatched),
            ),
            PmJournalAuthenticatedClassificationV1::OutOfProfile
            | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown => {
                (PmJournalCancelOutcomeV1::AmbiguousTimeout, None)
            }
        };
        let value = Self {
            auth_prepared_sequence,
            auth_grant_sequence,
            auth_result_sequence,
            prior_goal_f_sequence,
            instrument,
            request_commitment: PmJournalFingerprintV1::from_bytes(request_commitment),
            fixed_order_id: PmJournalFingerprintV1::from_bytes(fixed_order_id),
            observed_order_id: observed_order_id.map(PmJournalFingerprintV1::from_bytes),
            classification,
            canonical: PmJournalCancelResultV1 {
                client_order,
                venue_order,
                outcome,
                reject_reason,
            },
        };
        value.validate_shape()?;
        Ok(value)
    }

    pub(crate) const fn auth_prepared_sequence(&self) -> u64 {
        self.auth_prepared_sequence
    }

    pub(crate) const fn auth_grant_sequence(&self) -> u64 {
        self.auth_grant_sequence
    }

    pub(crate) const fn auth_result_sequence(&self) -> u64 {
        self.auth_result_sequence
    }

    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        self.prior_goal_f_sequence
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentId {
        self.instrument
    }

    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        self.request_commitment.bytes()
    }

    pub(crate) const fn fixed_order_id(&self) -> [u8; 32] {
        self.fixed_order_id.bytes()
    }

    pub(crate) const fn observed_order_id(&self) -> Option<[u8; 32]> {
        match self.observed_order_id {
            Some(identity) => Some(identity.bytes()),
            None => None,
        }
    }

    pub(crate) const fn classification(&self) -> PmJournalAuthenticatedClassificationV1 {
        self.classification
    }

    pub(crate) const fn canonical(&self) -> PmJournalCancelResultV1 {
        self.canonical
    }

    pub(crate) fn validate(self, scope: &PmJournalScopeV1) -> Result<(), PmJournalSchemaError> {
        self.canonical.validate(scope)?;
        if self.instrument != scope.instrument() {
            return Err(PmJournalSchemaError::RecordOutsideScope);
        }
        self.validate_shape()
    }

    fn validate_shape(self) -> Result<(), PmJournalSchemaError> {
        validate_sequence_chain(
            self.auth_prepared_sequence,
            self.auth_grant_sequence,
            self.auth_result_sequence,
            self.prior_goal_f_sequence,
        )?;
        if exact_order_bytes(self.canonical.venue_order) != Some(self.fixed_order_id.bytes()) {
            return Err(PmJournalSchemaError::InvalidAuthenticatedOrderIdentity);
        }
        let observed_matches_fixed = self.observed_order_id == Some(self.fixed_order_id);
        let valid = match self.classification {
            PmJournalAuthenticatedClassificationV1::Accepted => {
                self.canonical.outcome == PmJournalCancelOutcomeV1::Accepted
                    && self.canonical.reject_reason.is_none()
                    && observed_matches_fixed
            }
            PmJournalAuthenticatedClassificationV1::Rejected => {
                rejected_cancel(
                    self.canonical,
                    PmJournalCancelRejectReasonV1::AuthenticatedVenueRejected,
                ) && observed_matches_fixed
            }
            PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched => {
                rejected_cancel(
                    self.canonical,
                    PmJournalCancelRejectReasonV1::AuthenticatedDefinitelyNotDispatched,
                ) && self.observed_order_id.is_none()
            }
            PmJournalAuthenticatedClassificationV1::OutOfProfile
            | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown => {
                self.canonical.outcome == PmJournalCancelOutcomeV1::AmbiguousTimeout
                    && self.canonical.reject_reason.is_none()
            }
        };
        if valid {
            Ok(())
        } else {
            Err(PmJournalSchemaError::InvalidAuthenticatedResult)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "the frozen Copy schema keeps exact place and cancel records inline; indirection would change the durable representation"
)]
pub enum PmJournalAuthenticatedResultV1 {
    Place(PmJournalAuthenticatedPlaceResultV1),
    Cancel(PmJournalAuthenticatedCancelResultV1),
}

impl PmJournalAuthenticatedResultV1 {
    pub(crate) const fn auth_result_sequence(self) -> u64 {
        match self {
            Self::Place(result) => result.auth_result_sequence(),
            Self::Cancel(result) => result.auth_result_sequence(),
        }
    }

    pub(crate) const fn client_order(self) -> PmClientOrderKey {
        match self {
            Self::Place(result) => result.canonical.client_order,
            Self::Cancel(result) => result.canonical.client_order,
        }
    }

    pub(crate) const fn prior_goal_f_sequence(self) -> u64 {
        match self {
            Self::Place(result) => result.prior_goal_f_sequence,
            Self::Cancel(result) => result.prior_goal_f_sequence,
        }
    }

    pub(crate) fn validate(&self, scope: &PmJournalScopeV1) -> Result<(), PmJournalSchemaError> {
        match self {
            Self::Place(result) => result.validate(scope),
            Self::Cancel(result) => result.validate(scope),
        }
    }
}

fn validate_sequence_chain(
    prepared: u64,
    grant: u64,
    result: u64,
    prior_goal_f: u64,
) -> Result<(), PmJournalSchemaError> {
    if prior_goal_f == 0 || prepared == 0 || prepared >= grant || grant >= result {
        Err(PmJournalSchemaError::InvalidAuthenticatedSequence)
    } else {
        Ok(())
    }
}

fn rejected_place(
    canonical: &PmJournalPlaceResultV1,
    expected: PmJournalPlaceRejectReasonV1,
) -> bool {
    canonical.outcome == PmJournalPlaceOutcomeV1::Rejected
        && canonical.reject_reason == Some(expected)
        && canonical.venue_order.is_none()
}

fn rejected_cancel(
    canonical: PmJournalCancelResultV1,
    expected: PmJournalCancelRejectReasonV1,
) -> bool {
    canonical.outcome == PmJournalCancelOutcomeV1::Rejected
        && canonical.reject_reason == Some(expected)
}

fn exact_order_bytes(order: PmVenueOrderKey) -> Option<[u8; 32]> {
    let value = order.id();
    let bytes = value.as_str().as_bytes();
    if bytes.len() != 66 || &bytes[..2] != b"0x" {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, output) in decoded.iter_mut().enumerate() {
        let high = decode_lower_hex(bytes[2 + index * 2])?;
        let low = decode_lower_hex(bytes[3 + index * 2])?;
        *output = (high << 4) | low;
    }
    Some(decoded)
}

const fn decode_lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
