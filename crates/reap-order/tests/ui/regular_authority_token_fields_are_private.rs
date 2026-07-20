use reap_order::{
    ApprovedRegularCancel, ApprovedRegularSubmit, PreparedRegularCancel, PreparedRegularSubmit,
    ReservedRegularSubmit,
};

fn approved_fields_are_private(submit: ApprovedRegularSubmit, cancel: ApprovedRegularCancel) {
    let ApprovedRegularSubmit {
        account_id: _,
        order: _,
        canonical_numbers: _,
        origin: _,
    } = submit;
    let ApprovedRegularCancel {
        account_id: _,
        symbol: _,
        client_order_id: _,
        reason: _,
    } = cancel;
}

fn prepared_fields_are_private(submit: PreparedRegularSubmit, cancel: PreparedRegularCancel) {
    let PreparedRegularSubmit {
        account_id: _,
        idempotency_key: _,
        client_order_id: _,
        order: _,
        canonical_numbers: _,
        trade_mode: _,
    } = submit;
    let PreparedRegularCancel {
        account_id: _,
        symbol: _,
        client_order_id: _,
        reason: _,
    } = cancel;
}

fn reserved_fields_are_private(reserved: ReservedRegularSubmit) {
    let ReservedRegularSubmit {
        account_id: _,
        client_order_id: _,
        order: _,
        canonical_numbers: _,
    } = reserved;
}

fn main() {}
