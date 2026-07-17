use reap_core::NewOrder;
use reap_order::{
    ApprovedRegularCancel, ApprovedRegularSubmit, CancelOrderTransportError,
    PreparedRegularCancel, PreparedRegularSubmit, RegularApprovalScope,
    RegularCommandDispatcher, RegularExecutionProfileSet, RegularSubmitCompletion,
    ReservedRegularSubmit,
};

fn require_clone<T: Clone>() {}

fn tokens_are_linear() {
    require_clone::<ApprovedRegularSubmit>();
    require_clone::<ApprovedRegularCancel>();
    require_clone::<PreparedRegularSubmit>();
    require_clone::<PreparedRegularCancel>();
    require_clone::<ReservedRegularSubmit>();
    require_clone::<RegularApprovalScope>();
    require_clone::<RegularExecutionProfileSet>();
    require_clone::<RegularSubmitCompletion>();
    require_clone::<CancelOrderTransportError>();
    require_clone::<RegularCommandDispatcher>();
}

fn approved_raw_parts_are_private(submit: ApprovedRegularSubmit, cancel: ApprovedRegularCancel) {
    let _ = submit.into_parts();
    let _ = cancel.into_parts();
}

fn reserved_raw_parts_are_private(reserved: ReservedRegularSubmit) {
    let _ = reserved.into_parts();
}

fn raw_values_do_not_promote(order: NewOrder, cancel: (String, String, String, String)) {
    let _: PreparedRegularSubmit = order.into();
    let _: PreparedRegularCancel = cancel.into();
}

fn main() {}
