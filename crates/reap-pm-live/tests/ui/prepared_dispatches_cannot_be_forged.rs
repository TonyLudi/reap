use reap_pm_live::{PmPreparedCancelDispatch, PmPreparedPlaceDispatch};

fn value<T>() -> T {
    panic!("compile-fail fixture")
}

fn main() {
    let _ = PmPreparedPlaceDispatch {
        journal_sequence: value(),
        request: value(),
    };
    let _ = PmPreparedCancelDispatch {
        journal_sequence: value(),
        request: value(),
    };
}
