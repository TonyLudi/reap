use reap_pm_core::{PmPrice, PmQuantity, PmTick, U256};

fn private_unchecked_constructors_cannot_be_called() {
    let _ = PmPrice::from_units_unchecked(1);
    let _ = PmTick::from_units_unchecked(100);
    let _ = PmQuantity::from_protocol_units_unchecked(U256::ONE);
}

fn floats_do_not_implicitly_promote(price: f64, quantity: f64) -> (PmPrice, PmQuantity) {
    (price.into(), quantity.into())
}

fn main() {}
