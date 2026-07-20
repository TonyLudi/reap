use reap_core::{OrderIntent, Side};
use reap_strategy::ChaosExecutionIntent;

fn require_clone<T: Clone>() {}

fn forge<T>() -> T {
    panic!()
}

fn typed_intents_are_linear() {
    require_clone::<ChaosExecutionIntent>();
}

fn private_constructors_cannot_be_called() {
    let _ = ChaosExecutionIntent::quote(
        "BTC-USDT".to_string(),
        Side::Buy,
        1.0,
        100.0,
        "forged quote".to_string(),
    );
    let _ = ChaosExecutionIntent::hedge(
        "BTC-USDT".to_string(),
        Side::Sell,
        1.0,
        100.0,
        "forged hedge".to_string(),
        forge(),
    );
    let _ = ChaosExecutionIntent::cancel_owned("client-1".to_string(), "forged cancel".to_string());
}

fn serialized_intents_do_not_promote(intent: OrderIntent) -> ChaosExecutionIntent {
    intent.into()
}

fn main() {}
