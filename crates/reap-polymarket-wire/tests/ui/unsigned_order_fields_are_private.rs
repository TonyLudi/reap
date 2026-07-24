use reap_pm_core::{EvmAddress, PmOrderSalt, PmOrderSide, PmTokenId, U256};
use reap_polymarket_wire::PmUnsignedClobV2Order;

fn main() {
    let _ = PmUnsignedClobV2Order {
        salt: todo::<PmOrderSalt>(),
        maker: todo::<EvmAddress>(),
        signer: todo::<EvmAddress>(),
        token_id: todo::<PmTokenId>(),
        maker_amount: U256::ONE,
        taker_amount: U256::ONE,
        side: PmOrderSide::Buy,
        timestamp_ms: 1,
    };
}

fn todo<T>() -> T {
    unreachable!()
}
