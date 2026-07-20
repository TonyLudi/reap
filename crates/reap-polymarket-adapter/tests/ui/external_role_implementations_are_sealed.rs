use reap_pm_core::{
    PmBookEvent, PmConnectionId, PmInstrumentHandle, PmMarketEvent, PmProductSource,
};
use reap_polymarket_adapter::PmPublicObservationRole;

struct ForgedPublicRole;

impl PmPublicObservationRole for ForgedPublicRole {
    type MarketObservation = PmMarketEvent;
    type BookObservation = PmBookEvent;

    fn instrument(&self) -> PmInstrumentHandle {
        unimplemented!()
    }

    fn source(&self) -> PmProductSource {
        unimplemented!()
    }

    fn connection(&self) -> PmConnectionId {
        unimplemented!()
    }
}

fn main() {}
