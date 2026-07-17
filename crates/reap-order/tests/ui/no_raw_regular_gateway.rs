use reap_core::NewOrder;
use reap_order::{OkxGatewayIo, OkxOrderGateway, PrivateStateReducer, RegularExecution};
use reap_venue::okx::{OkxCancelOrder, OkxPlaceOrder};

async fn raw_gateway_entrypoints(
    gateway: &mut OkxOrderGateway,
    io: &OkxGatewayIo,
    state: &mut PrivateStateReducer,
    order: NewOrder,
) {
    let _ = gateway.submit("decision", order.clone()).await;
    let _ = gateway
        .submit_registered("decision", order.clone(), state)
        .await;
    let _ = gateway.prepare_submit("decision", order.clone());
    let _ = gateway.prepare_registered_submit("decision", order, "client-1");
    let _ = gateway
        .cancel("BTC-USDT", None, Some("client-1".to_string()))
        .await;
    let _ = io
        .cancel("BTC-USDT", None, Some("client-1".to_string()))
        .await;
    let _ = gateway.prepare_cancel((
        "account".to_string(),
        "BTC-USDT".to_string(),
        "client-1".to_string(),
        "reason".to_string(),
    ));
}

fn raw_transports_are_rejected(
    gateway: &mut OkxOrderGateway,
    execution: &dyn RegularExecution,
    place: &OkxPlaceOrder,
    cancel: &OkxCancelOrder,
) {
    let _ = gateway.set_order_transport(Box::new(()));
    let _ = RegularExecution::place_regular_order(execution, place);
    let _ = RegularExecution::cancel_regular_order(execution, cancel);
    let _ = RegularExecution::cancel_regular_order_via_rest(execution, cancel);
}

fn main() {}
