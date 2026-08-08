//! Controlled read-ingress teardown gates and retained failure evidence.

use thiserror::Error;

use super::{
    PmBookTaskFailure, PmHttpTaskFailure, PmPublicTaskFailure, PmReadIngressActorError,
    PmUserTaskFailure,
};

pub(super) const fn book_shutdown_ready(
    sockets_drained: bool,
    pending_book: bool,
    pending_book_demand: bool,
) -> bool {
    sockets_drained && !pending_book && !pending_book_demand
}

pub(super) const fn http_shutdown_ready(
    book_stopped: bool,
    sockets_drained: bool,
    pending_http: bool,
    refresh_quiescent: bool,
    read_lanes_quiescent: bool,
) -> bool {
    book_stopped && sockets_drained && !pending_http && refresh_quiescent && read_lanes_quiescent
}

#[derive(Debug, Error)]
#[error(
    "authenticated read-ingress shutdown failed: actor={actor:?}, public={public:?}, user={user:?}, http={http:?}, book={book:?}, read_unresolved_counts={read_unresolved_counts:?}, refresh_unresolved={refresh_unresolved}, book_obligation_unresolved={book_obligation_unresolved}, timed_out={timed_out}"
)]
pub(in crate::composition::product::authenticated_loopback) struct PmReadIngressShutdownError {
    pub(super) actor: Option<Box<PmReadIngressActorError>>,
    pub(super) public: Option<PmPublicTaskFailure>,
    pub(super) user: Option<PmUserTaskFailure>,
    pub(super) http: Option<PmHttpTaskFailure>,
    pub(super) book: Option<PmBookTaskFailure>,
    /// Public, private, and reconciliation lanes followed by retained private
    /// and retained reconciliation admissions.
    pub(super) read_unresolved_counts: [usize; 5],
    pub(super) refresh_unresolved: bool,
    pub(super) book_obligation_unresolved: bool,
    pub(super) timed_out: bool,
}
