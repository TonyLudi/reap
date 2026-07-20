mod pm_state {
    use reap_pm_core::{PmPrice, PmQuantity, U256};

    pub fn unchecked_constructors_are_not_consumer_authority() {
        let _ = PmPrice::from_units_unchecked(400_000);
        let _ = PmQuantity::from_protocol_units_unchecked(U256::ONE);
    }

    pub fn private_fields_are_not_consumer_authority(
        price: PmPrice,
        quantity: PmQuantity,
    ) {
        let _ = price.units;
        let _ = quantity.protocol_units;
    }
}

fn main() {}
