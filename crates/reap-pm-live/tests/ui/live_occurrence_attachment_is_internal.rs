use reap_pm_live::{PmFixtureQueryOccurrence, PmLiveOpenOrdersInput};
use reap_polymarket_live_adapter::PmCompleteOpenOrdersCut;

fn attach_complete_cut_to_caller_occurrence(
    occurrence: PmFixtureQueryOccurrence,
    cut: PmCompleteOpenOrdersCut,
) {
    let _ = PmLiveOpenOrdersInput::new(occurrence, cut);
}

fn main() {}
