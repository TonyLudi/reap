use reap_pm_live::{
    ApprovedPmCancel, ApprovedPmQuote, PreparedPmCancel, PreparedPmQuote, ReservedPmCancel,
    ReservedPmQuote,
};

fn consume<T>(_authority: T) {}

fn approved_quote(authority: ApprovedPmQuote) {
    consume(authority);
    consume(authority);
}

fn reserved_quote(authority: ReservedPmQuote) {
    consume(authority);
    consume(authority);
}

fn prepared_quote(authority: PreparedPmQuote) {
    consume(authority);
    consume(authority);
}

fn approved_cancel(authority: ApprovedPmCancel) {
    consume(authority);
    consume(authority);
}

fn reserved_cancel(authority: ReservedPmCancel) {
    consume(authority);
    consume(authority);
}

fn prepared_cancel(authority: PreparedPmCancel) {
    consume(authority);
    consume(authority);
}

fn main() {}
