use reap_pm_core::PmClientOrderId;

fn raw_identity_bytes_cannot_be_recovered(client_order_id: PmClientOrderId) {
    let _ = client_order_id.0;
}

fn main() {}
