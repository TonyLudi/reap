use reap_order::{
    ApprovedRegularSubmit, ClientOrderIdGenerator, GeneratedClientOrderId, OwnedRegularOrders,
    PrivateStateReducer,
};

fn require_clone<T: Clone>() {}

fn generated_client_order_id_is_not_cloneable() {
    require_clone::<GeneratedClientOrderId>();
}

fn generator_constructor_is_private() {
    let _ = ClientOrderIdGenerator::new("forged", 7);
}

fn raw_string_cannot_reserve(
    owned: &mut OwnedRegularOrders,
    approved: ApprovedRegularSubmit,
    private_state: &mut PrivateStateReducer,
) {
    let _ = owned.reserve_local(
        approved,
        "forged-client-order-id".to_string(),
        private_state,
        0,
    );
}

fn main() {}
