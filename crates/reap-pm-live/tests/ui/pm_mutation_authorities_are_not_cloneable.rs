use reap_pm_live::{
    ApprovedPmCancel, ApprovedPmQuote, PreparedPmCancel, PreparedPmQuote, ReservedPmCancel,
    ReservedPmQuote,
};

fn approved_quote(authority: ApprovedPmQuote) {
    let _ = authority.clone();
}

fn reserved_quote(authority: ReservedPmQuote) {
    let _ = authority.clone();
}

fn prepared_quote(authority: PreparedPmQuote) {
    let _ = authority.clone();
}

fn approved_cancel(authority: ApprovedPmCancel) {
    let _ = authority.clone();
}

fn reserved_cancel(authority: ReservedPmCancel) {
    let _ = authority.clone();
}

fn prepared_cancel(authority: PreparedPmCancel) {
    let _ = authority.clone();
}

fn main() {}
