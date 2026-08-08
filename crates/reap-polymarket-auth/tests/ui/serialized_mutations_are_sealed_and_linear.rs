use reap_polymarket_auth::{
    L2Credentials, L2Timestamp, SerializedOwnedCancelRequest, SerializedPlaceRequest,
    SignedClobV2Order,
};

fn tamper_place(
    _credentials: &L2Credentials,
    _timestamp: L2Timestamp,
    mut body: SerializedPlaceRequest,
) {
    body.body[0] ^= 1;
}

fn reuse_place(
    credentials: &L2Credentials,
    timestamp: L2Timestamp,
    body: SerializedPlaceRequest,
) {
    let _first = credentials.authenticate_place(timestamp, body);
    let _second = credentials.authenticate_place(timestamp, body);
}

fn tamper_cancel(
    _credentials: &L2Credentials,
    _timestamp: L2Timestamp,
    mut body: SerializedOwnedCancelRequest,
) {
    body.body[0] ^= 1;
}

fn reuse_cancel(
    credentials: &L2Credentials,
    timestamp: L2Timestamp,
    body: SerializedOwnedCancelRequest,
) {
    let _first = credentials.authenticate_owned_cancel(timestamp, body);
    let _second = credentials.authenticate_owned_cancel(timestamp, body);
}

fn reserialize_place(credentials: &L2Credentials, signed: SignedClobV2Order) {
    let _first = credentials.serialize_gtc_post_only(signed);
    let _second = credentials.serialize_gtc_post_only(signed);
}

fn main() {}
