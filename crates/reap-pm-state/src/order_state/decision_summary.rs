use reap_pm_core::{PmOrderIdentity, U256};

use super::{
    OrderEntry, OwnershipState, PmOrderState, PmReservationKnowledge, PmReservationTotalsError,
};

/// One uncached exact summary used by a single quote decision.
///
/// This carrier is crate-private and has no public constructor, so callers
/// cannot forge or retain exposure facts across state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmOrderDecisionSummary {
    unmanaged_ambiguity: bool,
    first_unknown_reservation: Option<PmOrderIdentity>,
    reservation_totals: Result<(U256, U256), PmReservationTotalsError>,
    live_count: u16,
    unresolved_count: u16,
}

impl PmOrderDecisionSummary {
    pub(crate) const fn has_unmanaged_ambiguity(self) -> bool {
        self.unmanaged_ambiguity
    }

    pub(crate) const fn first_unknown_reservation(self) -> Option<PmOrderIdentity> {
        self.first_unknown_reservation
    }

    pub(crate) const fn reservation_totals(self) -> Result<(U256, U256), PmReservationTotalsError> {
        self.reservation_totals
    }

    pub(crate) const fn live_count(self) -> u16 {
        self.live_count
    }

    pub(crate) const fn unresolved_count(self) -> u16 {
        self.unresolved_count
    }
}

impl PmOrderState {
    pub(crate) fn decision_summary(&self) -> PmOrderDecisionSummary {
        if self.live_count == 0 {
            return PmOrderDecisionSummary {
                unmanaged_ambiguity: false,
                first_unknown_reservation: None,
                reservation_totals: Ok((U256::ZERO, U256::ZERO)),
                live_count: 0,
                unresolved_count: 0,
            };
        }
        let (dense_summary, canonical_required) = summarize(self.entries.iter());
        if canonical_required {
            summarize(self.canonical_entries()).0
        } else {
            dense_summary
        }
    }
}

fn summarize<'a>(entries: impl Iterator<Item = &'a OrderEntry>) -> (PmOrderDecisionSummary, bool) {
    let mut unmanaged_ambiguity = false;
    let mut first_unknown_reservation = None;
    let mut reservation_totals = Ok((U256::ZERO, U256::ZERO));
    let mut live_count = 0_u16;
    let mut unresolved_count = 0_u16;
    let mut canonical_required = false;

    for entry in entries.filter(|entry| entry.is_live()) {
        live_count = live_count
            .checked_add(1)
            .expect("configured order capacity fits u16");
        let ambiguous = entry.ownership == OwnershipState::Ambiguous;
        unmanaged_ambiguity |= ambiguous;
        let unknown = entry.reservation == Some(PmReservationKnowledge::Unknown);
        canonical_required |= ambiguous || unknown;
        if ambiguous || unknown {
            first_unknown_reservation.get_or_insert(entry.identity);
        }
        if ambiguous || entry.event.is_none() || entry.missing_from_complete_open_snapshot {
            unresolved_count = unresolved_count
                .checked_add(1)
                .expect("configured order capacity fits u16");
        }
        let Ok((collateral, outcome)) = reservation_totals else {
            continue;
        };
        reservation_totals = match entry.reservation {
            None => Ok((collateral, outcome)),
            Some(PmReservationKnowledge::Unknown) => {
                Err(PmReservationTotalsError::Unknown(entry.identity))
            }
            Some(PmReservationKnowledge::Known(reservation)) => collateral
                .checked_add(reservation.collateral())
                .and_then(|collateral| {
                    outcome
                        .checked_add(reservation.outcome())
                        .map(|outcome| (collateral, outcome))
                })
                .map_err(|_| PmReservationTotalsError::Overflow),
        };
    }

    (
        PmOrderDecisionSummary {
            unmanaged_ambiguity,
            first_unknown_reservation,
            reservation_totals,
            live_count,
            unresolved_count,
        },
        canonical_required,
    )
}
