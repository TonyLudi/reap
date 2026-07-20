use reap_pm_core::{PmOrderAmounts, U256};
use serde::Deserialize;

fn require_deserialize<T: for<'de> Deserialize<'de>>() {}

fn raw_units_do_not_promote(maker: U256, taker: U256) -> PmOrderAmounts {
    (maker, taker).into()
}

fn serde_cannot_mint_derived_order_amounts() {
    require_deserialize::<PmOrderAmounts>();
}

fn main() {}
