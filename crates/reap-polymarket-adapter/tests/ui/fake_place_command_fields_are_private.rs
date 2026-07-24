use reap_pm_core::{PmAccountScope, PmClientOrderKey, PmOrderSide, PmPrice, PmQuantity};
use reap_polymarket_adapter::{PmFakePlaceCommand, PmFixtureInstrumentScope, PmGtcPostOnlyProfile};
use reap_polymarket_wire::PmUnsignedClobV2Order;

fn main() {
    let _ = PmFakePlaceCommand {
        account_scope: todo::<PmAccountScope>(),
        instrument_scope: todo::<PmFixtureInstrumentScope>(),
        client_order: todo::<PmClientOrderKey>(),
        side: PmOrderSide::Buy,
        price: todo::<PmPrice>(),
        quantity: todo::<PmQuantity>(),
        unsigned_order: todo::<PmUnsignedClobV2Order>(),
        profile: todo::<PmGtcPostOnlyProfile>(),
    };
}

fn todo<T>() -> T {
    unreachable!()
}
