//! Move-only authenticated completions admitted after the result barrier.

#[cfg(any(test, feature = "loopback-evidence"))]
use reap_pm_core::PmVenueOrderId;
use reap_pm_core::{PmClientOrderKey, PmVenueOrderKey};
use reap_pm_state::PmOwnedCancelIntent;
#[cfg(any(test, feature = "loopback-evidence"))]
use thiserror::Error;

#[cfg(any(test, feature = "loopback-evidence"))]
use crate::authenticated_journal::{
    PmAuthenticatedCancelResultAcknowledged, PmAuthenticatedCancelResultKindV1,
    PmAuthenticatedPlaceResultAcknowledged, PmAuthenticatedPlaceResultKindV1,
};
#[cfg(any(test, feature = "loopback-evidence"))]
use crate::journal::PmJournalAuthenticatedClassificationV1;
use crate::journal::{PmJournalAuthenticatedCancelResultV1, PmJournalAuthenticatedPlaceResultV1};

/// One place completion whose authenticated result is already durable.
///
/// This value is deliberately not `Clone` or `Copy`; the scheduler must move
/// it into the critical lane exactly once.
#[derive(Debug)]
pub(crate) struct PmLivePlaceCompletion {
    result: PmJournalAuthenticatedPlaceResultV1,
}

impl PmLivePlaceCompletion {
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn after_durable_result(
        acknowledged: PmAuthenticatedPlaceResultAcknowledged,
    ) -> Result<Self, PmLiveCompletionError> {
        let (grant, result_sequence, authenticated) = acknowledged.into_parts();
        if authenticated.client_order() != grant.client_order()
            || authenticated.instrument() != grant.instrument()
            || authenticated.grant_sequence() != grant.grant_sequence()
        {
            return Err(PmLiveCompletionError::AuthResultCorrelationMismatch);
        }
        let classification = place_classification(authenticated.outcome());
        let observed = authenticated.observed_order_id();
        let accepted_venue_order =
            if classification == PmJournalAuthenticatedClassificationV1::Accepted {
                let observed = observed.ok_or(PmLiveCompletionError::AuthResultShapeMismatch)?;
                if observed != grant.expected_order_id() {
                    return Err(PmLiveCompletionError::AuthResultShapeMismatch);
                }
                Some(exact_venue_order(grant.client_order(), observed)?)
            } else {
                None
            };
        let result = PmJournalAuthenticatedPlaceResultV1::new(
            grant.prepared_sequence(),
            grant.grant_sequence(),
            result_sequence,
            grant.prior_goal_f_sequence(),
            grant.client_order(),
            grant.instrument(),
            grant.request_commitment(),
            grant.expected_order_id(),
            observed,
            accepted_venue_order,
            classification,
        )
        .map_err(|_| PmLiveCompletionError::AuthResultShapeMismatch)?;
        Ok(Self { result })
    }

    pub(crate) const fn client_order(&self) -> PmClientOrderKey {
        self.result.canonical().client_order
    }

    pub(crate) const fn result(&self) -> PmJournalAuthenticatedPlaceResultV1 {
        self.result
    }

    pub(crate) const fn auth_result_sequence(&self) -> u64 {
        self.result.auth_result_sequence()
    }

    pub(crate) const fn into_result(self) -> PmJournalAuthenticatedPlaceResultV1 {
        self.result
    }

    #[cfg(test)]
    pub(crate) const fn from_journal_for_scheduler_test(
        result: PmJournalAuthenticatedPlaceResultV1,
    ) -> Self {
        Self { result }
    }
}

/// One exact-owned cancel completion whose authenticated result is durable.
///
/// The original state-owned cancel intent remains attached until the sole
/// canonical reducer consumes both values.
#[derive(Debug)]
pub(crate) struct PmLiveCancelCompletion {
    intent: PmOwnedCancelIntent,
    result: PmJournalAuthenticatedCancelResultV1,
}

impl PmLiveCancelCompletion {
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn after_durable_result(
        intent: PmOwnedCancelIntent,
        acknowledged: PmAuthenticatedCancelResultAcknowledged,
    ) -> Result<Self, PmLiveCompletionError> {
        let (grant, result_sequence, authenticated) = acknowledged.into_parts();
        if authenticated.client_order() != grant.client_order()
            || authenticated.instrument() != grant.instrument()
            || authenticated.venue_order() != grant.venue_order()
            || authenticated.grant_sequence() != grant.grant_sequence()
            || intent.client_order() != grant.client_order()
            || intent.venue_order() != grant.venue_order()
        {
            return Err(PmLiveCompletionError::AuthResultCorrelationMismatch);
        }
        let result = PmJournalAuthenticatedCancelResultV1::new(
            grant.prepared_sequence(),
            grant.grant_sequence(),
            result_sequence,
            grant.prior_goal_f_sequence(),
            grant.client_order(),
            grant.instrument(),
            grant.venue_order(),
            grant.request_commitment(),
            grant.fixed_order_id(),
            authenticated.observed_order_id(),
            cancel_classification(authenticated.outcome()),
        )
        .map_err(|_| PmLiveCompletionError::AuthResultShapeMismatch)?;
        let canonical = result.canonical();
        if canonical.client_order != intent.client_order()
            || canonical.venue_order != intent.venue_order()
        {
            return Err(PmLiveCompletionError::CancelOwnershipMismatch);
        }
        Ok(Self { intent, result })
    }

    pub(crate) const fn client_order(&self) -> PmClientOrderKey {
        self.intent.client_order()
    }

    pub(crate) const fn venue_order(&self) -> PmVenueOrderKey {
        self.intent.venue_order()
    }

    pub(crate) const fn result(&self) -> PmJournalAuthenticatedCancelResultV1 {
        self.result
    }

    pub(crate) const fn auth_result_sequence(&self) -> u64 {
        self.result.auth_result_sequence()
    }

    pub(crate) const fn intent(&self) -> PmOwnedCancelIntent {
        self.intent
    }

    pub(crate) const fn into_parts(
        self,
    ) -> (PmOwnedCancelIntent, PmJournalAuthenticatedCancelResultV1) {
        (self.intent, self.result)
    }

    #[cfg(test)]
    pub(crate) const fn from_journal_for_scheduler_test(
        intent: PmOwnedCancelIntent,
        result: PmJournalAuthenticatedCancelResultV1,
    ) -> Self {
        Self { intent, result }
    }
}

/// Move-only product-owner proof that the second durable barrier completed
/// and canonical reduction ran. Only the mutation owner can construct it;
/// workers consume it to release their one in-flight purpose slot.
#[derive(Debug)]
pub(crate) enum PmAuthenticatedBridgeApplied {
    Place {
        #[cfg_attr(
            not(any(test, feature = "loopback-evidence")),
            allow(
                dead_code,
                reason = "default Goal-F persistence carries this identity for an authenticated supervisor that is not compiled"
            )
        )]
        client_order: PmClientOrderKey,
        #[cfg_attr(
            not(any(test, feature = "loopback-evidence")),
            allow(
                dead_code,
                reason = "default Goal-F persistence carries this identity for an authenticated supervisor that is not compiled"
            )
        )]
        auth_result_sequence: u64,
    },
    Cancel {
        #[cfg_attr(
            not(any(test, feature = "loopback-evidence")),
            allow(
                dead_code,
                reason = "default Goal-F persistence carries this identity for an authenticated supervisor that is not compiled"
            )
        )]
        client_order: PmClientOrderKey,
        #[cfg_attr(
            not(any(test, feature = "loopback-evidence")),
            allow(
                dead_code,
                reason = "default Goal-F persistence carries this identity for an authenticated supervisor that is not compiled"
            )
        )]
        auth_result_sequence: u64,
    },
}

impl PmAuthenticatedBridgeApplied {
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) const fn is_place(&self) -> bool {
        matches!(self, Self::Place { .. })
    }

    pub(super) const fn place(completion: &PmLivePlaceCompletion) -> Self {
        Self::Place {
            client_order: completion.client_order(),
            auth_result_sequence: completion.auth_result_sequence(),
        }
    }

    pub(super) const fn cancel(completion: &PmLiveCancelCompletion) -> Self {
        Self::Cancel {
            client_order: completion.client_order(),
            auth_result_sequence: completion.auth_result_sequence(),
        }
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) const fn into_identity(self) -> (bool, PmClientOrderKey, u64) {
        match self {
            Self::Place {
                client_order,
                auth_result_sequence,
            } => (true, client_order, auth_result_sequence),
            Self::Cancel {
                client_order,
                auth_result_sequence,
            } => (false, client_order, auth_result_sequence),
        }
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
#[allow(
    clippy::enum_variant_names,
    reason = "the mismatch suffix names three distinct authenticated correlation checks at the completion boundary"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PmLiveCompletionError {
    #[error("authenticated result acknowledgement does not match its exact durable send grant")]
    AuthResultCorrelationMismatch,
    #[error("authenticated result acknowledgement has a contradictory fixed-profile shape")]
    AuthResultShapeMismatch,
    #[error("authenticated cancel completion lost exact local ownership")]
    CancelOwnershipMismatch,
}

#[cfg(any(test, feature = "loopback-evidence"))]
const fn place_classification(
    outcome: PmAuthenticatedPlaceResultKindV1,
) -> PmJournalAuthenticatedClassificationV1 {
    match outcome {
        PmAuthenticatedPlaceResultKindV1::Accepted => {
            PmJournalAuthenticatedClassificationV1::Accepted
        }
        PmAuthenticatedPlaceResultKindV1::Rejected => {
            PmJournalAuthenticatedClassificationV1::Rejected
        }
        PmAuthenticatedPlaceResultKindV1::DefinitelyNotDispatched => {
            PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched
        }
        PmAuthenticatedPlaceResultKindV1::OutOfProfile => {
            PmJournalAuthenticatedClassificationV1::OutOfProfile
        }
        PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown => {
            PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown
        }
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
const fn cancel_classification(
    outcome: PmAuthenticatedCancelResultKindV1,
) -> PmJournalAuthenticatedClassificationV1 {
    match outcome {
        PmAuthenticatedCancelResultKindV1::Accepted => {
            PmJournalAuthenticatedClassificationV1::Accepted
        }
        PmAuthenticatedCancelResultKindV1::Rejected => {
            PmJournalAuthenticatedClassificationV1::Rejected
        }
        PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched => {
            PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched
        }
        PmAuthenticatedCancelResultKindV1::OutOfProfile => {
            PmJournalAuthenticatedClassificationV1::OutOfProfile
        }
        PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown => {
            PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown
        }
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn exact_venue_order(
    client_order: PmClientOrderKey,
    bytes: [u8; 32],
) -> Result<PmVenueOrderKey, PmLiveCompletionError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut identity = String::with_capacity(66);
    identity.push_str("0x");
    for byte in bytes {
        identity.push(char::from(HEX[usize::from(byte >> 4)]));
        identity.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    let id = PmVenueOrderId::new(&identity)
        .map_err(|_| PmLiveCompletionError::AuthResultShapeMismatch)?;
    Ok(PmVenueOrderKey::new(client_order.account(), id))
}
