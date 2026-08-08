use reap_pm_live::{PmPreparedCancelDispatch, PmPreparedPlaceDispatch};

fn consume<T>(_dispatch: T) {}

fn place(dispatch: PmPreparedPlaceDispatch) {
    consume(dispatch);
    consume(dispatch);
}

fn cancel(dispatch: PmPreparedCancelDispatch) {
    consume(dispatch);
    consume(dispatch);
}

fn main() {}
