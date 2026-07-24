use reap_benchmark_allocator::MeasurementWindow;
use reap_pm_core::{PmProductSource, PmSourceHandle};
use reap_pm_state::{
    MAX_PM_REFRESH_OBLIGATIONS, PmPrivateState, PmPrivateStateConfig, PmRefreshAdmission,
    PmRefreshReason, PmRefreshRequired, PmRefreshTicket,
};

use super::{DirectMechanismObservation, OverloadCaseId, measure_case};
use crate::coordinator::{Phase6RefreshAllocationProbe, PmCoordinatorError};
use crate::evidence::{account_scope, instrument, market_metadata, risk_limits};

pub(super) fn measure_reconciliation_refresh(
    window: &mut MeasurementWindow,
) -> DirectMechanismObservation {
    let tickets = refresh_tickets();
    let mut probe = Phase6RefreshAllocationProbe::new();
    let mut attempts = 0;
    let mut retained = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;

    measure_case(window, OverloadCaseId::ReconciliationRefreshEffects, || {
        for (index, ticket) in tickets
            .iter()
            .take(MAX_PM_REFRESH_OBLIGATIONS)
            .copied()
            .enumerate()
        {
            attempts += 1;
            probe
                .retain(
                    ticket,
                    u64::try_from(index + 1).expect("bounded refresh admission time"),
                )
                .expect("first 128 refresh obligations");
            retained += 1;
        }
        attempts += 1;
        assert!(matches!(
            probe.retain(tickets[MAX_PM_REFRESH_OBLIGATIONS], 129),
            Err(PmCoordinatorError::RefreshRetentionSaturated)
        ));
        rejected += 1;
        fail_closed_transitions += 1;
    });

    assert_eq!(probe.len(), MAX_PM_REFRESH_OBLIGATIONS);
    DirectMechanismObservation::new(
        OverloadCaseId::ReconciliationRefreshEffects,
        attempts,
        retained,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn refresh_tickets() -> Vec<PmRefreshTicket> {
    let scope = account_scope();
    let source =
        PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(2), scope.handle());
    let config = PmPrivateStateConfig::new(source, scope, instrument(), market_metadata())
        .expect("fixed refresh allocation configuration");
    let mut state =
        PmPrivateState::new(config, risk_limits()).expect("fixed refresh allocation state");
    let PmRefreshRequired::Inserted { ticket } = state
        .require_refresh(PmRefreshReason::FillObserved)
        .expect("first refresh requirement")
    else {
        panic!("first refresh requirement must insert");
    };
    assert_eq!(
        state
            .mark_refresh_admitted(ticket)
            .expect("first refresh admission"),
        PmRefreshAdmission::Admitted(ticket)
    );

    let mut tickets = Vec::with_capacity(MAX_PM_REFRESH_OBLIGATIONS + 1);
    tickets.push(ticket);
    for _ in 0..MAX_PM_REFRESH_OBLIGATIONS {
        let PmRefreshRequired::SupersededInFlight {
            required,
            in_flight,
        } = state
            .require_refresh(PmRefreshReason::FillObserved)
            .expect("superseded refresh requirement")
        else {
            panic!("in-flight refresh must issue a newer generation");
        };
        assert_eq!(in_flight, ticket);
        tickets.push(required);
    }
    tickets
}
