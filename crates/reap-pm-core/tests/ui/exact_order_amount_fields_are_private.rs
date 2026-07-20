use reap_pm_core::{PmOrderAmounts, U256};

fn private_maker_taker_fields_cannot_be_forged() {
    let _ = PmOrderAmounts {
        maker: U256::ONE,
        taker: U256::ONE,
    };
}

fn main() {}
