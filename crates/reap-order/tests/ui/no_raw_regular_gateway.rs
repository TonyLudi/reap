use reap_core::NewOrder;
use reap_order::{
    OkxGatewayIo, OkxOrderGateway, OkxOrderTransport, PrivateStateReducer, RegularExecution,
};
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
    transport: &dyn OkxOrderTransport,
    execution: &dyn RegularExecution,
    place: &OkxPlaceOrder,
    cancel: &OkxCancelOrder,
) {
    let _ = OkxOrderTransport::place_order(transport, place);
    let _ = OkxOrderTransport::cancel_order(transport, cancel);
    let _ = RegularExecution::cancel_regular_order(execution, cancel);
}

fn main() {}
