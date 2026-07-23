use reap_pm_live::{PmAgedLaneFailure, PmLaneKind, PmServiceTurnError, SaturationAction};

fn forge() -> PmServiceTurnError {
    PmServiceTurnError::Aged(PmAgedLaneFailure {
        lane: PmLaneKind::Public,
        action: SaturationAction::InvalidateStreamAndResync,
        evidence: None,
    })
}

fn main() {}
