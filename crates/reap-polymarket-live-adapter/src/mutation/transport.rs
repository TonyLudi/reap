//! Protocol authority: current official CLOB place and cancel documentation at
//! <https://docs.polymarket.com/trading/place-orders> and
//! <https://docs.polymarket.com/trading/manage-orders>. Pinned Predarb object
//! `8222273a9c72033b760e1d2fec813bc77144556d` independently corroborates only
//! the fixed POST/DELETE `/order` routes and exact owned-cancel body key.

use std::{fmt, time::Duration};

#[cfg(any(test, feature = "loopback-evidence"))]
use std::net::IpAddr;

use reap_pm_core::{PmVenueOrderId, U256};
use reap_polymarket_auth::{
    ExpectedOrderId, FixedOrderId, OwnedCancelSemanticRequestCommitment,
    PlaceSemanticRequestCommitment, RuntimeExactBodyCommitment,
};
use reap_polymarket_wire::{parse_live_cancel_result, parse_live_place_result};
use reqwest::{
    Client, Request, StatusCode, Url,
    header::{ACCEPT, CONNECTION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use zeroize::Zeroizing;

use super::{
    PmMutationEdgeError, PmRetainedOwnedCancelRequest, PmRetainedPlaceRequest,
    retained::RetainedL2Headers,
};

#[cfg(any(test, feature = "loopback-evidence"))]
const MAX_MUTATION_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const MAX_MUTATION_RESPONSE_BYTES: usize = 32 * 1_024;
const POLY_ADDRESS: HeaderName = HeaderName::from_static("poly_address");
const POLY_SIGNATURE: HeaderName = HeaderName::from_static("poly_signature");
const POLY_TIMESTAMP: HeaderName = HeaderName::from_static("poly_timestamp");
const POLY_API_KEY: HeaderName = HeaderName::from_static("poly_api_key");
const POLY_PASSPHRASE: HeaderName = HeaderName::from_static("poly_passphrase");

/// Literal-loopback-only configuration for mutation evidence. There is no
/// production or arbitrary-origin constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLoopbackMutationConfig {
    origin: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
}

impl PmLoopbackMutationConfig {
    /// Construct the evidence-only mutation transport. This API is absent from
    /// the default production build.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmMutationEdgeError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin = validate_loopback_origin(origin)?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
        })
    }
}

/// Exhaustive lifecycle-oriented classification for one mutation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmMutationClassification {
    DefinitelyNotDispatched,
    Accepted,
    Rejected,
    OutOfProfile,
    AcknowledgementUnknown,
}

/// Bounded, non-secret reason code retained for reconciliation and evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmMutationDiagnosticKind {
    PreSendValidation,
    AcceptedProfile,
    VenueRejected,
    ResponseIdentityMismatch,
    ResponseProfileMismatch,
    Redirect,
    AuthenticationInvalid,
    ReconciliationRequiredStatus,
    UnexpectedHttpStatus,
    MalformedResponse,
    ResponseTooLarge,
    RequestTimeout,
    TransportFailure,
    ResponseBodyTimeout,
    ResponseBodyFailure,
}

/// Non-secret bounded evidence about one classified attempt. Raw request or
/// response bytes and auth values are never exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmMutationDiagnostic {
    kind: PmMutationDiagnosticKind,
    http_status: Option<u16>,
    response_bytes: usize,
}

impl PmMutationDiagnostic {
    #[must_use]
    pub const fn kind(&self) -> PmMutationDiagnosticKind {
        self.kind
    }

    #[must_use]
    pub const fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    #[must_use]
    pub const fn response_bytes(&self) -> usize {
        self.response_bytes
    }

    const fn pre_send() -> Self {
        Self {
            kind: PmMutationDiagnosticKind::PreSendValidation,
            http_status: None,
            response_bytes: 0,
        }
    }

    const fn transport(kind: PmMutationDiagnosticKind) -> Self {
        Self {
            kind,
            http_status: None,
            response_bytes: 0,
        }
    }

    const fn response(
        kind: PmMutationDiagnosticKind,
        status: StatusCode,
        response_bytes: usize,
    ) -> Self {
        Self {
            kind,
            http_status: Some(status.as_u16()),
            response_bytes,
        }
    }
}

/// Classified place result with distinct runtime exact-body correlation and
/// secret-free semantic request identity.
///
/// This transport outcome is intentionally not serializable. Its
/// `runtime_exact_body_commitment` is secret-derived and must remain in memory;
/// upper layers may separately consume the semantic identity when constructing
/// their own durable records.
pub struct PmPlaceMutationOutcome {
    classification: PmMutationClassification,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: PlaceSemanticRequestCommitment,
    expected_order_id: ExpectedOrderId,
    observed_order_id: Option<PmVenueOrderId>,
    diagnostic: PmMutationDiagnostic,
    rejection_reason: Option<Zeroizing<String>>,
}

impl PmPlaceMutationOutcome {
    #[must_use]
    pub const fn classification(&self) -> PmMutationClassification {
        self.classification
    }

    #[must_use]
    pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment {
        self.runtime_exact_body_commitment
    }

    #[must_use]
    pub const fn semantic_request_commitment(&self) -> PlaceSemanticRequestCommitment {
        self.semantic_request_commitment
    }

    #[must_use]
    pub const fn expected_order_id(&self) -> ExpectedOrderId {
        self.expected_order_id
    }

    #[must_use]
    pub const fn observed_order_id(&self) -> Option<PmVenueOrderId> {
        self.observed_order_id
    }

    #[must_use]
    pub const fn diagnostic(&self) -> PmMutationDiagnostic {
        self.diagnostic
    }

    #[must_use]
    pub fn rejection_reason(&self) -> Option<&str> {
        self.rejection_reason.as_deref().map(String::as_str)
    }
}

impl fmt::Debug for PmPlaceMutationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPlaceMutationOutcome")
            .field("classification", &self.classification)
            .field("runtime_exact_body_commitment", &"[REDACTED; NON_DURABLE]")
            .field(
                "semantic_request_commitment",
                &self.semantic_request_commitment,
            )
            .field("expected_order_id", &self.expected_order_id)
            .field("observed_order_id", &self.observed_order_id)
            .field("diagnostic", &self.diagnostic)
            .field(
                "rejection_reason",
                &self.rejection_reason.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Classified exact-owned cancel result with distinct runtime exact-body
/// correlation and secret-free semantic request identity.
///
/// This transport outcome is intentionally not serializable. The runtime
/// commitment is retained only for exact post-send correlation and must never
/// enter durable storage.
pub struct PmCancelMutationOutcome {
    classification: PmMutationClassification,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: OwnedCancelSemanticRequestCommitment,
    order_id: FixedOrderId,
    observed_order_id: Option<PmVenueOrderId>,
    diagnostic: PmMutationDiagnostic,
    rejection_reason: Option<Zeroizing<String>>,
}

impl PmCancelMutationOutcome {
    #[must_use]
    pub const fn classification(&self) -> PmMutationClassification {
        self.classification
    }

    #[must_use]
    pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment {
        self.runtime_exact_body_commitment
    }

    #[must_use]
    pub const fn semantic_request_commitment(&self) -> OwnedCancelSemanticRequestCommitment {
        self.semantic_request_commitment
    }

    #[must_use]
    pub const fn order_id(&self) -> FixedOrderId {
        self.order_id
    }

    #[must_use]
    pub const fn observed_order_id(&self) -> Option<PmVenueOrderId> {
        self.observed_order_id
    }

    #[must_use]
    pub const fn diagnostic(&self) -> PmMutationDiagnostic {
        self.diagnostic
    }

    #[must_use]
    pub fn rejection_reason(&self) -> Option<&str> {
        self.rejection_reason.as_deref().map(String::as_str)
    }
}

impl fmt::Debug for PmCancelMutationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmCancelMutationOutcome")
            .field("classification", &self.classification)
            .field("runtime_exact_body_commitment", &"[REDACTED; NON_DURABLE]")
            .field(
                "semantic_request_commitment",
                &self.semantic_request_commitment,
            )
            .field("order_id", &self.order_id)
            .field("observed_order_id", &self.observed_order_id)
            .field("diagnostic", &self.diagnostic)
            .field(
                "rejection_reason",
                &self.rejection_reason.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// The only fixed place transport role. Construction still requires the
/// feature-gated loopback configuration, so this type grants no production
/// order-entry path.
pub struct PmFixedPlaceLoopbackRole {
    edge: MutationHttpEdge,
}

impl PmFixedPlaceLoopbackRole {
    pub fn new(config: PmLoopbackMutationConfig) -> Result<Self, PmMutationEdgeError> {
        Ok(Self {
            edge: MutationHttpEdge::new(config)?,
        })
    }

    /// Consume one retained request and perform exactly one POST `/order`.
    pub async fn send(&mut self, request: PmRetainedPlaceRequest) -> PmPlaceMutationOutcome {
        let evidence = PlaceEvidence {
            runtime_exact_body_commitment: request.runtime_exact_body_commitment(),
            semantic_request_commitment: request.semantic_request_commitment(),
            expected_order_id: request.expected_order_id(),
            expected_making_amount: request.expected_making_amount(),
            expected_taking_amount: request.expected_taking_amount(),
        };
        let prepared = match self.edge.prepare_place(&request) {
            Ok(prepared) => prepared,
            Err(()) => return place_pre_send_failure(evidence),
        };
        classify_place_observation(evidence, self.edge.execute(prepared).await)
    }
}

impl fmt::Debug for PmFixedPlaceLoopbackRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmFixedPlaceLoopbackRole([LOOPBACK])")
    }
}

/// The only exact-owned cancel transport role. It has no cancel-all, market,
/// batch, or arbitrary-order-ID operation.
pub struct PmExactOwnedCancelLoopbackRole {
    edge: MutationHttpEdge,
}

impl PmExactOwnedCancelLoopbackRole {
    pub fn new(config: PmLoopbackMutationConfig) -> Result<Self, PmMutationEdgeError> {
        Ok(Self {
            edge: MutationHttpEdge::new(config)?,
        })
    }

    /// Consume one retained request and perform exactly one DELETE `/order`.
    pub async fn send(&mut self, request: PmRetainedOwnedCancelRequest) -> PmCancelMutationOutcome {
        let evidence = CancelEvidence {
            runtime_exact_body_commitment: request.runtime_exact_body_commitment(),
            semantic_request_commitment: request.semantic_request_commitment(),
            order_id: request.order_id(),
        };
        let prepared = match self.edge.prepare_cancel(&request) {
            Ok(prepared) => prepared,
            Err(()) => return cancel_pre_send_failure(evidence),
        };
        classify_cancel_observation(evidence, self.edge.execute(prepared).await)
    }
}

impl fmt::Debug for PmExactOwnedCancelLoopbackRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmExactOwnedCancelLoopbackRole([LOOPBACK])")
    }
}

struct MutationHttpEdge {
    client: Client,
    origin: Url,
}

impl MutationHttpEdge {
    fn new(config: PmLoopbackMutationConfig) -> Result<Self, PmMutationEdgeError> {
        let client = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            // A mutation has crossed the may-have-sent boundary once execute
            // begins. Even reqwest's otherwise safe protocol-NACK retry must
            // therefore be disabled explicitly rather than inferred from the
            // current HTTP/1-only configuration.
            .retry(reqwest::retry::never())
            .redirect(Policy::none())
            .no_proxy()
            .http1_only()
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| PmMutationEdgeError::TransportBuild)?;
        Ok(Self {
            client,
            origin: config.origin,
        })
    }

    fn prepare_place(&self, request: &PmRetainedPlaceRequest) -> Result<Request, ()> {
        if !request.remains_valid() {
            return Err(());
        }
        let mut url = self.origin.clone();
        url.set_path("/order");
        self.client
            .post(url)
            .headers(fixed_headers(&request.headers)?)
            .body(request.body.as_slice().to_vec())
            .build()
            .map_err(|_| ())
    }

    fn prepare_cancel(&self, request: &PmRetainedOwnedCancelRequest) -> Result<Request, ()> {
        if !request.remains_valid() {
            return Err(());
        }
        let mut url = self.origin.clone();
        url.set_path("/order");
        self.client
            .delete(url)
            .headers(fixed_headers(&request.headers)?)
            .body(request.body.as_slice().to_vec())
            .build()
            .map_err(|_| ())
    }

    async fn execute(&self, request: Request) -> MutationHttpObservation {
        let mut response = match self.client.execute(request).await {
            Ok(response) => response,
            Err(error) => {
                return MutationHttpObservation::Fault(if error.is_timeout() {
                    PmMutationDiagnostic::transport(PmMutationDiagnosticKind::RequestTimeout)
                } else {
                    PmMutationDiagnostic::transport(PmMutationDiagnosticKind::TransportFailure)
                });
            }
        };
        let status = response.status();
        if status.is_redirection() {
            return MutationHttpObservation::Redirect(status);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MUTATION_RESPONSE_BYTES as u64)
        {
            return MutationHttpObservation::Fault(PmMutationDiagnostic::response(
                PmMutationDiagnosticKind::ResponseTooLarge,
                status,
                0,
            ));
        }

        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(MAX_MUTATION_RESPONSE_BYTES);
        let mut body = Zeroizing::new(Vec::with_capacity(capacity));
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    let Some(next_length) = body.len().checked_add(chunk.len()) else {
                        return MutationHttpObservation::Fault(PmMutationDiagnostic::response(
                            PmMutationDiagnosticKind::ResponseTooLarge,
                            status,
                            body.len(),
                        ));
                    };
                    if next_length > MAX_MUTATION_RESPONSE_BYTES {
                        return MutationHttpObservation::Fault(PmMutationDiagnostic::response(
                            PmMutationDiagnosticKind::ResponseTooLarge,
                            status,
                            body.len(),
                        ));
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) => {
                    let kind = if error.is_timeout() {
                        PmMutationDiagnosticKind::ResponseBodyTimeout
                    } else {
                        PmMutationDiagnosticKind::ResponseBodyFailure
                    };
                    return MutationHttpObservation::Fault(PmMutationDiagnostic::response(
                        kind,
                        status,
                        body.len(),
                    ));
                }
            }
        }
        MutationHttpObservation::Complete { status, body }
    }
}

enum MutationHttpObservation {
    Complete {
        status: StatusCode,
        body: Zeroizing<Vec<u8>>,
    },
    Redirect(StatusCode),
    Fault(PmMutationDiagnostic),
}

#[derive(Clone, Copy)]
struct PlaceEvidence {
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: PlaceSemanticRequestCommitment,
    expected_order_id: ExpectedOrderId,
    expected_making_amount: U256,
    expected_taking_amount: U256,
}

#[derive(Clone, Copy)]
struct CancelEvidence {
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: OwnedCancelSemanticRequestCommitment,
    order_id: FixedOrderId,
}

fn classify_place_observation(
    evidence: PlaceEvidence,
    observation: MutationHttpObservation,
) -> PmPlaceMutationOutcome {
    match observation {
        MutationHttpObservation::Fault(diagnostic) => place_outcome(
            evidence,
            PmMutationClassification::AcknowledgementUnknown,
            None,
            diagnostic,
            None,
        ),
        MutationHttpObservation::Redirect(status) => place_outcome(
            evidence,
            PmMutationClassification::OutOfProfile,
            None,
            PmMutationDiagnostic::response(PmMutationDiagnosticKind::Redirect, status, 0),
            None,
        ),
        MutationHttpObservation::Complete { status, body } => {
            classify_complete_place(evidence, status, body.as_slice())
        }
    }
}

fn classify_complete_place(
    evidence: PlaceEvidence,
    status: StatusCode,
    body: &[u8],
) -> PmPlaceMutationOutcome {
    let response_bytes = body.len();
    let parsed = match parse_live_place_result(body) {
        Ok(parsed) => parsed,
        Err(_) => return place_status_fallback(evidence, status, response_bytes),
    };
    let observed = parsed.order_id();

    if status_requires_reconciliation(status) {
        return place_outcome(
            evidence,
            PmMutationClassification::OutOfProfile,
            observed,
            PmMutationDiagnostic::response(status_diagnostic(status), status, response_bytes),
            parsed.error_message(),
        );
    }
    if status == StatusCode::REQUEST_TIMEOUT || status.is_server_error() {
        return place_outcome(
            evidence,
            PmMutationClassification::AcknowledgementUnknown,
            observed,
            PmMutationDiagnostic::response(
                PmMutationDiagnosticKind::UnexpectedHttpStatus,
                status,
                response_bytes,
            ),
            parsed.error_message(),
        );
    }
    if !parsed.success() {
        if status == StatusCode::OK
            || status == StatusCode::BAD_REQUEST
            || status == StatusCode::UNPROCESSABLE_ENTITY
        {
            return place_outcome(
                evidence,
                PmMutationClassification::Rejected,
                None,
                PmMutationDiagnostic::response(
                    PmMutationDiagnosticKind::VenueRejected,
                    status,
                    response_bytes,
                ),
                parsed.error_message(),
            );
        }
        return place_status_fallback(evidence, status, response_bytes);
    }
    if status != StatusCode::OK {
        return place_outcome(
            evidence,
            PmMutationClassification::OutOfProfile,
            observed,
            PmMutationDiagnostic::response(
                PmMutationDiagnosticKind::UnexpectedHttpStatus,
                status,
                response_bytes,
            ),
            None,
        );
    }

    let exact_identity = observed.is_some_and(|id| {
        let expected = evidence.expected_order_id.to_string();
        id.as_str() == expected
    });
    if !exact_identity {
        return place_outcome(
            evidence,
            PmMutationClassification::OutOfProfile,
            observed,
            PmMutationDiagnostic::response(
                PmMutationDiagnosticKind::ResponseIdentityMismatch,
                status,
                response_bytes,
            ),
            None,
        );
    }
    if parsed.status() != "live"
        || parsed.making_amount() != Some(evidence.expected_making_amount)
        || parsed.taking_amount() != Some(evidence.expected_taking_amount)
        || !parsed.trade_ids().is_empty()
        || !parsed.transaction_hashes().is_empty()
    {
        return place_outcome(
            evidence,
            PmMutationClassification::OutOfProfile,
            observed,
            PmMutationDiagnostic::response(
                PmMutationDiagnosticKind::ResponseProfileMismatch,
                status,
                response_bytes,
            ),
            None,
        );
    }
    place_outcome(
        evidence,
        PmMutationClassification::Accepted,
        observed,
        PmMutationDiagnostic::response(
            PmMutationDiagnosticKind::AcceptedProfile,
            status,
            response_bytes,
        ),
        None,
    )
}

fn classify_cancel_observation(
    evidence: CancelEvidence,
    observation: MutationHttpObservation,
) -> PmCancelMutationOutcome {
    match observation {
        MutationHttpObservation::Fault(diagnostic) => cancel_outcome(
            evidence,
            PmMutationClassification::AcknowledgementUnknown,
            None,
            diagnostic,
            None,
        ),
        MutationHttpObservation::Redirect(status) => cancel_outcome(
            evidence,
            PmMutationClassification::OutOfProfile,
            None,
            PmMutationDiagnostic::response(PmMutationDiagnosticKind::Redirect, status, 0),
            None,
        ),
        MutationHttpObservation::Complete { status, body } => {
            classify_complete_cancel(evidence, status, body.as_slice())
        }
    }
}

fn classify_complete_cancel(
    evidence: CancelEvidence,
    status: StatusCode,
    body: &[u8],
) -> PmCancelMutationOutcome {
    let response_bytes = body.len();
    let parsed = match parse_live_cancel_result(body) {
        Ok(parsed) => parsed,
        Err(_) => return cancel_status_fallback(evidence, status, response_bytes),
    };
    let observed = single_cancel_identity(&parsed);

    if status_requires_reconciliation(status) {
        return cancel_outcome(
            evidence,
            PmMutationClassification::OutOfProfile,
            observed,
            PmMutationDiagnostic::response(status_diagnostic(status), status, response_bytes),
            None,
        );
    }
    if status == StatusCode::REQUEST_TIMEOUT || status.is_server_error() {
        return cancel_outcome(
            evidence,
            PmMutationClassification::AcknowledgementUnknown,
            observed,
            PmMutationDiagnostic::response(
                PmMutationDiagnosticKind::UnexpectedHttpStatus,
                status,
                response_bytes,
            ),
            None,
        );
    }

    let expected = evidence.order_id.to_string();
    let accepted = parsed.canceled().len() == 1
        && parsed.canceled()[0].as_str() == expected
        && parsed.not_canceled().is_empty();
    let rejected = parsed.canceled().is_empty()
        && parsed.not_canceled().len() == 1
        && parsed.not_canceled()[0].0.as_str() == expected;
    if accepted && status == StatusCode::OK {
        return cancel_outcome(
            evidence,
            PmMutationClassification::Accepted,
            observed,
            PmMutationDiagnostic::response(
                PmMutationDiagnosticKind::AcceptedProfile,
                status,
                response_bytes,
            ),
            None,
        );
    }
    if rejected
        && (status == StatusCode::OK
            || status == StatusCode::BAD_REQUEST
            || status == StatusCode::UNPROCESSABLE_ENTITY)
    {
        return cancel_outcome(
            evidence,
            PmMutationClassification::Rejected,
            observed,
            PmMutationDiagnostic::response(
                PmMutationDiagnosticKind::VenueRejected,
                status,
                response_bytes,
            ),
            Some(parsed.not_canceled()[0].1.as_str()),
        );
    }
    cancel_outcome(
        evidence,
        PmMutationClassification::OutOfProfile,
        observed,
        PmMutationDiagnostic::response(
            if observed.is_some_and(|id| id.as_str() != expected) {
                PmMutationDiagnosticKind::ResponseIdentityMismatch
            } else {
                PmMutationDiagnosticKind::ResponseProfileMismatch
            },
            status,
            response_bytes,
        ),
        None,
    )
}

fn place_status_fallback(
    evidence: PlaceEvidence,
    status: StatusCode,
    response_bytes: usize,
) -> PmPlaceMutationOutcome {
    let (classification, kind) = fallback_status_classification(status);
    place_outcome(
        evidence,
        classification,
        None,
        PmMutationDiagnostic::response(kind, status, response_bytes),
        None,
    )
}

fn cancel_status_fallback(
    evidence: CancelEvidence,
    status: StatusCode,
    response_bytes: usize,
) -> PmCancelMutationOutcome {
    let (classification, kind) = fallback_status_classification(status);
    cancel_outcome(
        evidence,
        classification,
        None,
        PmMutationDiagnostic::response(kind, status, response_bytes),
        None,
    )
}

fn fallback_status_classification(
    status: StatusCode,
) -> (PmMutationClassification, PmMutationDiagnosticKind) {
    if status_requires_reconciliation(status) {
        (
            PmMutationClassification::OutOfProfile,
            status_diagnostic(status),
        )
    } else {
        (
            PmMutationClassification::AcknowledgementUnknown,
            if status == StatusCode::OK {
                PmMutationDiagnosticKind::MalformedResponse
            } else {
                PmMutationDiagnosticKind::UnexpectedHttpStatus
            },
        )
    }
}

fn status_requires_reconciliation(status: StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403 | 409 | 425 | 429)
}

fn status_diagnostic(status: StatusCode) -> PmMutationDiagnosticKind {
    if matches!(status.as_u16(), 401 | 403) {
        PmMutationDiagnosticKind::AuthenticationInvalid
    } else {
        PmMutationDiagnosticKind::ReconciliationRequiredStatus
    }
}

fn single_cancel_identity(
    parsed: &reap_polymarket_wire::PmLiveCancelResult,
) -> Option<PmVenueOrderId> {
    if parsed.canceled().len() + parsed.not_canceled().len() != 1 {
        return None;
    }
    parsed
        .canceled()
        .first()
        .copied()
        .or_else(|| parsed.not_canceled().first().map(|(id, _)| *id))
}

fn place_pre_send_failure(evidence: PlaceEvidence) -> PmPlaceMutationOutcome {
    place_outcome(
        evidence,
        PmMutationClassification::DefinitelyNotDispatched,
        None,
        PmMutationDiagnostic::pre_send(),
        None,
    )
}

fn cancel_pre_send_failure(evidence: CancelEvidence) -> PmCancelMutationOutcome {
    cancel_outcome(
        evidence,
        PmMutationClassification::DefinitelyNotDispatched,
        None,
        PmMutationDiagnostic::pre_send(),
        None,
    )
}

fn place_outcome(
    evidence: PlaceEvidence,
    classification: PmMutationClassification,
    observed_order_id: Option<PmVenueOrderId>,
    diagnostic: PmMutationDiagnostic,
    rejection_reason: Option<&str>,
) -> PmPlaceMutationOutcome {
    PmPlaceMutationOutcome {
        classification,
        runtime_exact_body_commitment: evidence.runtime_exact_body_commitment,
        semantic_request_commitment: evidence.semantic_request_commitment,
        expected_order_id: evidence.expected_order_id,
        observed_order_id,
        diagnostic,
        rejection_reason: rejection_reason.map(|reason| Zeroizing::new(reason.to_owned())),
    }
}

fn cancel_outcome(
    evidence: CancelEvidence,
    classification: PmMutationClassification,
    observed_order_id: Option<PmVenueOrderId>,
    diagnostic: PmMutationDiagnostic,
    rejection_reason: Option<&str>,
) -> PmCancelMutationOutcome {
    PmCancelMutationOutcome {
        classification,
        runtime_exact_body_commitment: evidence.runtime_exact_body_commitment,
        semantic_request_commitment: evidence.semantic_request_commitment,
        order_id: evidence.order_id,
        observed_order_id,
        diagnostic,
        rejection_reason: rejection_reason.map(|reason| Zeroizing::new(reason.to_owned())),
    }
}

fn fixed_headers(headers: &RetainedL2Headers) -> Result<HeaderMap, ()> {
    let mut map = HeaderMap::with_capacity(8);
    map.insert(ACCEPT, HeaderValue::from_static("application/json"));
    map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    map.insert(CONNECTION, HeaderValue::from_static("close"));
    insert_sensitive(&mut map, POLY_ADDRESS, headers.address.as_str())?;
    insert_sensitive(&mut map, POLY_SIGNATURE, headers.signature.as_str())?;
    insert_sensitive(&mut map, POLY_TIMESTAMP, headers.timestamp.as_str())?;
    insert_sensitive(&mut map, POLY_API_KEY, headers.api_key.as_str())?;
    insert_sensitive(&mut map, POLY_PASSPHRASE, headers.passphrase.as_str())?;
    Ok(map)
}

fn insert_sensitive(map: &mut HeaderMap, name: HeaderName, value: &str) -> Result<(), ()> {
    let mut value = HeaderValue::from_str(value).map_err(|_| ())?;
    value.set_sensitive(true);
    map.insert(name, value);
    Ok(())
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_timeouts(
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<(), PmMutationEdgeError> {
    if connect_timeout.is_zero() || request_timeout.is_zero() {
        return Err(PmMutationEdgeError::InvalidLoopbackConfiguration(
            "connect and request timeouts must be positive",
        ));
    }
    if connect_timeout > MAX_MUTATION_TIMEOUT || request_timeout > MAX_MUTATION_TIMEOUT {
        return Err(PmMutationEdgeError::InvalidLoopbackConfiguration(
            "connect and request timeouts must not exceed 60 seconds",
        ));
    }
    Ok(())
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_loopback_origin(origin: &str) -> Result<Url, PmMutationEdgeError> {
    let url = Url::parse(origin).map_err(|_| {
        PmMutationEdgeError::InvalidLoopbackConfiguration("origin URL is malformed")
    })?;
    if url.scheme() != "http" {
        return Err(PmMutationEdgeError::InvalidLoopbackConfiguration(
            "loopback evidence origin must use HTTP",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PmMutationEdgeError::InvalidLoopbackConfiguration(
            "loopback evidence origin must not contain user information",
        ));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(PmMutationEdgeError::InvalidLoopbackConfiguration(
            "loopback evidence origin must use exact root path",
        ));
    }
    let host = url
        .host_str()
        .ok_or(PmMutationEdgeError::InvalidLoopbackConfiguration(
            "loopback evidence origin must contain a host",
        ))?;
    if !host
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
    {
        return Err(PmMutationEdgeError::InvalidLoopbackConfiguration(
            "loopback evidence origin must use a literal loopback address",
        ));
    }
    if url.port().is_none() {
        return Err(PmMutationEdgeError::InvalidLoopbackConfiguration(
            "loopback evidence origin must use an explicit port",
        ));
    }
    Ok(url)
}
