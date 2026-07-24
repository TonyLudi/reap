use reap_pm_core::{PmInstrumentHandle, PmInstrumentId, PmOrderSide, PmPrice, PmQuantity, U256};
use reap_pm_strategy::PmValidatedQuoteCandidate;

fn main() {
    let _ = PmValidatedQuoteCandidate {
        instrument: todo::<PmInstrumentHandle>(),
        instrument_id: todo::<PmInstrumentId>(),
        side: PmOrderSide::Buy,
        price: todo::<PmPrice>(),
        quantity: todo::<PmQuantity>(),
        maker_amount: U256::ONE,
        taker_amount: U256::ONE,
    };
}

fn todo<T>() -> T {
    unreachable!()
}
