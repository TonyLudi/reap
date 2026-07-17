use reap_live::{CancelAction, CoordinatorOutput, LiveAction, ReconcileAction, SubmitAction};

fn inspect_internal_actions(output: CoordinatorOutput) {
    let _ = output.actions;
}

fn main() {}
