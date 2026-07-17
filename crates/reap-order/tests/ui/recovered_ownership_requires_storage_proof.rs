use reap_order::{OwnedRegularOrders, RegularExecutionPolicy};
use reap_storage::{ProvenRegularOrderBinding, ProvenRegularSubmitRequest};

fn require_clone<T: Clone>() {}

fn raw_identity_cannot_register(owned: &mut OwnedRegularOrders, policy: &RegularExecutionPolicy) {
    let _ = owned.register_recovered(policy, "account", "BTC-USDT", "foreign-order");
}

fn storage_proof_fields_are_private() {
    let _ = ProvenRegularSubmitRequest {
        account_id: "account".to_string(),
        symbol: "BTC-USDT".to_string(),
        client_order_id: "foreign-order".to_string(),
        idempotency_key: "forged".to_string(),
    };
}

fn recovery_authority_is_linear() {
    require_clone::<ProvenRegularSubmitRequest>();
    require_clone::<ProvenRegularOrderBinding>();
}

fn main() {}
