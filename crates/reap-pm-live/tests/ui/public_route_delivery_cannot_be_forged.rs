use reap_pm_core::{PmMarketEvent, ReceivedEventEnvelope};
use reap_pm_live::PmPublicMetadataDelivery;

fn forge(envelope: ReceivedEventEnvelope<PmMarketEvent>) -> PmPublicMetadataDelivery {
    PmPublicMetadataDelivery { envelope }
}

fn main() {}
