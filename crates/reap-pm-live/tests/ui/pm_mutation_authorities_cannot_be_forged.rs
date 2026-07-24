use reap_pm_live::{
    ApprovedPmCancel, ApprovedPmQuote, PreparedPmCancel, PreparedPmQuote, ReservedPmCancel,
    ReservedPmQuote,
};

fn value<T>() -> T {
    panic!("compile-fail fixture")
}

fn forge() {
    let _ = ApprovedPmQuote { facts: value() };
    let _ = ReservedPmQuote { facts: value() };
    let _ = PreparedPmQuote {
        facts: value(),
        journal_sequence: 1,
        command: value(),
    };
    let _ = ApprovedPmCancel { facts: value() };
    let _ = ReservedPmCancel { facts: value() };
    let _ = PreparedPmCancel {
        facts: value(),
        journal_sequence: 1,
        command: value(),
    };
}

fn main() {}
