//! Feature-gated signer and L2 custody for authenticated loopback evidence.
//!
//! This is deliberately a distinct owner from the production-capable
//! read-only [`crate::PmPrivateConnectivityOwner`]. It exists only in tests or
//! with `loopback-evidence`, and its retained mutation outputs can reach only
//! the literal-loopback mutation transports exposed under the same feature.

use std::{fmt, sync::Arc};

use reap_pm_core::{
    PmAccountScope, PmConfigurationFingerprint, PmGoalFTradingDomain, PmInstrumentHandle,
    PmPublicObservationGrant, PmSpenderDomain, exact_order_amounts,
};
use reap_polymarket_adapter::{
    PmCancelOwnedPurpose, PmExactOwnedCancelRequest, PmFixedOrderType, PmGtcPostOnlyPlaceRequest,
    PmGtcPostOnlyProfile,
};
use reap_polymarket_auth::{
    AuthenticatedJournalCredentialSlotFingerprint, CredentialSlotId, FixedEoaSigner, FixedOrderId,
    L2Credentials, L2Timestamp, PmAuthError, PmClobDomain,
};
use reap_polymarket_wire::{PmClobV2SignatureType, PmWireScope};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::{
    PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole, PmAuthorizedMutationServerTime,
    PmCredentialAuthoritySupervisor, PmLiveAdapterError, PmMutationEdgeError, PmPrivateHttpConfig,
    PmRetainedOwnedCancelRequest, PmRetainedPlaceRequest, PmUserWsConfig,
    config::OriginMode,
    private_credentials::{
        CredentialRequest, PmHttpCredentialRole, PmUserWsCredentialRole, handle_credential_request,
    },
    private_http::PmPrivateHttpTransport,
};

const MUTATION_AUTHORITY_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmLoopbackMutationAuthError {
    #[error("loopback mutation authority configuration is invalid: {0}")]
    InvalidConfiguration(&'static str),
    #[error("place request does not match the once-bound account/instrument/domain")]
    PlaceScopeMismatch,
    #[error("place request does not match the once-bound GTC/post-only profile")]
    PlaceProfileMismatch,
    #[error("place request unsigned order contradicts its admitted neutral facts")]
    PlaceOrderMismatch,
    #[error("cancel request does not match the once-bound account/instrument")]
    CancelScopeMismatch,
    #[error("cancel request does not carry the once-bound exact-owned purpose")]
    CancelPurposeMismatch,
    #[error("loopback mutation authority channel is saturated")]
    AuthoritySaturated,
    #[error("loopback mutation authority is unavailable")]
    AuthorityClosed,
    #[error(transparent)]
    Live(#[from] PmLiveAdapterError),
    #[error(transparent)]
    Auth(#[from] PmAuthError),
    #[error(transparent)]
    Retention(#[from] PmMutationEdgeError),
}

/// A place request that could not become an authenticated Prepared value.
///
/// The exact neutral request remains take-once so its already-durable Goal-F
/// authority cannot disappear when channel admission, validation, signing,
/// serialization, authentication, retention, or a reported authority closure
/// returns an error to the caller.
pub struct PmLoopbackPlaceAuthenticationFailure {
    reason: PmLoopbackMutationAuthError,
    request: PmGtcPostOnlyPlaceRequest,
}

impl PmLoopbackPlaceAuthenticationFailure {
    #[must_use]
    pub const fn reason(&self) -> PmLoopbackMutationAuthError {
        self.reason
    }

    #[must_use]
    pub fn into_request(self) -> PmGtcPostOnlyPlaceRequest {
        self.request
    }
}

impl fmt::Debug for PmLoopbackPlaceAuthenticationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmLoopbackPlaceAuthenticationFailure")
            .field("reason", &self.reason)
            .field("request", &"<retained>")
            .finish()
    }
}

impl fmt::Display for PmLoopbackPlaceAuthenticationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(formatter)
    }
}

impl std::error::Error for PmLoopbackPlaceAuthenticationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.reason)
    }
}

/// A cancel request that could not become an authenticated Prepared value.
pub struct PmLoopbackCancelAuthenticationFailure {
    reason: PmLoopbackMutationAuthError,
    request: PmExactOwnedCancelRequest,
}

impl PmLoopbackCancelAuthenticationFailure {
    #[must_use]
    pub const fn reason(&self) -> PmLoopbackMutationAuthError {
        self.reason
    }

    #[must_use]
    pub fn into_request(self) -> PmExactOwnedCancelRequest {
        self.request
    }
}

impl fmt::Debug for PmLoopbackCancelAuthenticationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmLoopbackCancelAuthenticationFailure")
            .field("reason", &self.reason)
            .field("request", &"<retained>")
            .finish()
    }
}

impl fmt::Display for PmLoopbackCancelAuthenticationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(formatter)
    }
}

impl std::error::Error for PmLoopbackCancelAuthenticationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.reason)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MutationBinding {
    account: PmAccountScope,
    instrument: PmInstrumentHandle,
    trading_domain: PmGoalFTradingDomain,
    clob_domain: PmClobDomain,
    place_profile: PmGtcPostOnlyProfile,
    cancel_purpose: PmCancelOwnedPurpose,
    signature_profile: PmClobV2SignatureType,
}

/// Test/loopback-only owner of the sole EOA signer and L2 bundle for one
/// account. Construction is statically absent from normal production builds.
pub struct PmLoopbackMutationConnectivityOwner {
    http_config: PmPrivateHttpConfig,
    user_ws_config: PmUserWsConfig,
    binding: MutationBinding,
    configuration_fingerprint: PmConfigurationFingerprint,
    credential_slot_fingerprint: AuthenticatedJournalCredentialSlotFingerprint,
    signer: FixedEoaSigner,
    credentials: L2Credentials,
}

impl PmLoopbackMutationConnectivityOwner {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        account: PmAccountScope,
        instrument: PmInstrumentHandle,
        trading_domain: PmGoalFTradingDomain,
        place_profile: PmGtcPostOnlyProfile,
        cancel_purpose: PmCancelOwnedPurpose,
        observation_grant: PmPublicObservationGrant,
        credential_slot: CredentialSlotId,
        signer: FixedEoaSigner,
        credentials: L2Credentials,
    ) -> Result<Self, PmLoopbackMutationAuthError> {
        Self::new_with_signature_profile(
            http_config,
            user_ws_config,
            account,
            instrument,
            trading_domain,
            place_profile,
            cancel_purpose,
            observation_grant,
            credential_slot,
            signer,
            credentials,
            PmClobV2SignatureType::Eoa,
        )
    }

    /// Construct the feature-gated PM-T2 proxy profile. The private key, L2
    /// credentials, and account signer remain one EOA; the account funder is
    /// the distinct proxy maker. This grants no production transport.
    #[allow(clippy::too_many_arguments)]
    pub fn new_pm_t2_proxy(
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        account: PmAccountScope,
        instrument: PmInstrumentHandle,
        trading_domain: PmGoalFTradingDomain,
        place_profile: PmGtcPostOnlyProfile,
        cancel_purpose: PmCancelOwnedPurpose,
        observation_grant: PmPublicObservationGrant,
        credential_slot: CredentialSlotId,
        signer: FixedEoaSigner,
        credentials: L2Credentials,
    ) -> Result<Self, PmLoopbackMutationAuthError> {
        Self::new_with_signature_profile(
            http_config,
            user_ws_config,
            account,
            instrument,
            trading_domain,
            place_profile,
            cancel_purpose,
            observation_grant,
            credential_slot,
            signer,
            credentials,
            PmClobV2SignatureType::Proxy,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_signature_profile(
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        account: PmAccountScope,
        instrument: PmInstrumentHandle,
        trading_domain: PmGoalFTradingDomain,
        place_profile: PmGtcPostOnlyProfile,
        cancel_purpose: PmCancelOwnedPurpose,
        observation_grant: PmPublicObservationGrant,
        credential_slot: CredentialSlotId,
        signer: FixedEoaSigner,
        credentials: L2Credentials,
        signature_profile: PmClobV2SignatureType,
    ) -> Result<Self, PmLoopbackMutationAuthError> {
        if http_config.mode() != OriginMode::LocalEvidence || user_ws_config.is_production() {
            return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                "loopback mutation authority requires loopback private HTTP and user WebSocket configurations",
            ));
        }
        let wire_scope = http_config.exact_order_scope();
        if wire_scope.condition() != user_ws_config.condition() {
            return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                "private HTTP and user WebSocket must bind the same condition",
            ));
        }
        if trading_domain.instrument().market() != wire_scope.market()
            || trading_domain.instrument().token() != wire_scope.token()
        {
            return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                "private HTTP scope must match the Goal-F trading domain",
            ));
        }
        if observation_grant.instrument() != instrument
            || observation_grant.polymarket_instrument() != trading_domain.instrument()
        {
            return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                "public observation grant must match the bound instrument and trading domain",
            ));
        }
        if account.chain() != trading_domain.chain() {
            return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                "account chain must match the Goal-F trading domain",
            ));
        }
        let configured_eoa = signer.address().as_core();
        let configured_funder = account.funder().address();
        match signature_profile {
            PmClobV2SignatureType::Eoa
                if credentials.address() != signer.address()
                    || account.signer().address() != configured_eoa
                    || configured_funder != configured_eoa =>
            {
                return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                    "signer, L2 credentials, account signer, and funder must be one EOA",
                ));
            }
            PmClobV2SignatureType::Proxy
                if credentials.address() != signer.address()
                    || account.signer().address() != configured_eoa =>
            {
                return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                    "private-key signer, L2 credentials, and account signer must be one EOA",
                ));
            }
            PmClobV2SignatureType::Proxy
                if configured_funder.bytes() == [0; 20]
                    || configured_eoa.bytes() == [0; 20]
                    || configured_funder == configured_eoa =>
            {
                return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                    "PM-T2 proxy funder must be nonzero and distinct from the signer EOA",
                ));
            }
            PmClobV2SignatureType::Eoa | PmClobV2SignatureType::Proxy => {}
        }
        if place_profile.order_type() != PmFixedOrderType::Gtc
            || !place_profile.post_only()
            || place_profile.defer_exec()
            || place_profile.expiration() != 0
        {
            return Err(PmLoopbackMutationAuthError::InvalidConfiguration(
                "only the Goal-F GTC/post-only profile is supported",
            ));
        }
        let clob_domain = match trading_domain.spender_domain() {
            PmSpenderDomain::Standard => PmClobDomain::Standard,
            PmSpenderDomain::NegativeRisk => PmClobDomain::NegativeRisk,
        };
        let credential_slot_fingerprint =
            credentials.authenticated_journal_credential_slot(credential_slot);
        Ok(Self {
            http_config,
            user_ws_config,
            binding: MutationBinding {
                account,
                instrument,
                trading_domain,
                clob_domain,
                place_profile,
                cancel_purpose,
                signature_profile,
            },
            configuration_fingerprint: observation_grant.configuration_fingerprint(),
            credential_slot_fingerprint,
            signer,
            credentials,
        })
    }

    /// Split once into read roles, independent place/cancel authentication
    /// roles, and the lifecycle supervisor for their one shared custody task.
    pub fn split(self) -> Result<PmLoopbackMutationConnectivityRoles, PmLoopbackMutationAuthError> {
        let address = self.credentials.address();
        let transport = PmPrivateHttpTransport::new(&self.http_config, address)?;
        let senders = spawn_loopback_authority(self.binding, self.signer, self.credentials)?;
        let balance_signature_type = match self.binding.signature_profile {
            PmClobV2SignatureType::Eoa => crate::PmReadOnlySignatureType::Eoa,
            PmClobV2SignatureType::Proxy => crate::PmReadOnlySignatureType::Proxy,
        };
        let http = PmAuthenticatedHttpOwner::from_authority_with_account_profile(
            transport,
            self.http_config.exact_order_scope(),
            address,
            self.binding.account.funder().address(),
            balance_signature_type,
            PmHttpCredentialRole::from_sender(senders.read.clone()),
        );
        let user_ws = PmAuthenticatedUserWsRole::from_authority(
            self.user_ws_config,
            PmUserWsCredentialRole::from_sender(senders.read),
        );
        Ok(PmLoopbackMutationConnectivityRoles {
            http,
            user_ws,
            place: PmLoopbackPlaceAuthenticationRole {
                sender: senders.place,
            },
            cancel: PmLoopbackCancelAuthenticationRole {
                sender: senders.cancel,
            },
            binding: PmLoopbackMutationConnectivityBinding {
                configuration_fingerprint: self.configuration_fingerprint,
                account: self.binding.account,
                instrument: self.binding.instrument,
                trading_domain: self.binding.trading_domain,
                wire_scope: self.http_config.exact_order_scope(),
                signature_profile: self.binding.signature_profile,
            },
            credential_slot_fingerprint: self.credential_slot_fingerprint,
            supervisor: senders.supervisor,
        })
    }
}

impl fmt::Debug for PmLoopbackMutationConnectivityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmLoopbackMutationConnectivityOwner([REDACTED])")
    }
}

pub struct PmLoopbackMutationConnectivityRoles {
    http: PmAuthenticatedHttpOwner,
    user_ws: PmAuthenticatedUserWsRole,
    place: PmLoopbackPlaceAuthenticationRole,
    cancel: PmLoopbackCancelAuthenticationRole,
    binding: PmLoopbackMutationConnectivityBinding,
    credential_slot_fingerprint: AuthenticatedJournalCredentialSlotFingerprint,
    supervisor: PmCredentialAuthoritySupervisor,
}

/// Move-only, non-secret proof of the complete scope bound by the sole
/// loopback signer/credential owner.
///
/// Construction remains private to that owner. The product startup gate
/// consumes this value and compares every field with its exact
/// `PmConnectivityConfig` before either journal is opened or a worker exists.
#[derive(Debug)]
pub struct PmLoopbackMutationConnectivityBinding {
    configuration_fingerprint: PmConfigurationFingerprint,
    account: PmAccountScope,
    instrument: PmInstrumentHandle,
    trading_domain: PmGoalFTradingDomain,
    wire_scope: PmWireScope,
    signature_profile: PmClobV2SignatureType,
}

impl PmLoopbackMutationConnectivityBinding {
    #[must_use]
    pub const fn configuration_fingerprint(&self) -> PmConfigurationFingerprint {
        self.configuration_fingerprint
    }

    #[must_use]
    pub const fn account(&self) -> PmAccountScope {
        self.account
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn trading_domain(&self) -> PmGoalFTradingDomain {
        self.trading_domain
    }

    #[must_use]
    pub const fn wire_scope(&self) -> PmWireScope {
        self.wire_scope
    }

    #[must_use]
    pub const fn signature_profile(&self) -> PmClobV2SignatureType {
        self.signature_profile
    }
}

impl PmLoopbackMutationConnectivityRoles {
    #[must_use]
    pub fn into_roles(
        self,
    ) -> (
        PmAuthenticatedHttpOwner,
        PmAuthenticatedUserWsRole,
        PmLoopbackPlaceAuthenticationRole,
        PmLoopbackCancelAuthenticationRole,
        PmLoopbackMutationConnectivityBinding,
        AuthenticatedJournalCredentialSlotFingerprint,
        PmCredentialAuthoritySupervisor,
    ) {
        (
            self.http,
            self.user_ws,
            self.place,
            self.cancel,
            self.binding,
            self.credential_slot_fingerprint,
            self.supervisor,
        )
    }
}

impl fmt::Debug for PmLoopbackMutationConnectivityRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmLoopbackMutationConnectivityRoles([REDACTED])")
    }
}

pub struct PmLoopbackPlaceAuthenticationRole {
    sender: mpsc::Sender<PlaceAuthenticationRequest>,
}

impl PmLoopbackPlaceAuthenticationRole {
    /// Authenticate one exact neutral request.
    ///
    /// # Cancellation contract
    ///
    /// The supervised authenticated worker MUST await this future to
    /// completion without selecting it against cancellation. Rust cannot
    /// return a moved request to a caller that drops its future.
    /// Process/task stop recovers durable Goal-F intent; it must never resend.
    pub async fn authenticate_place(
        &mut self,
        request: PmGtcPostOnlyPlaceRequest,
        server_time: PmAuthorizedMutationServerTime,
    ) -> Result<PmRetainedPlaceRequest, PmLoopbackPlaceAuthenticationFailure> {
        let request = Arc::new(request);
        let timestamp = server_time.into_l2_timestamp();
        let (response, receive) = oneshot::channel();
        if let Err(error) = self.sender.try_send(PlaceAuthenticationRequest {
            request: Arc::clone(&request),
            timestamp,
            response,
        }) {
            let reason = classify_place_send_error(error);
            return Err(retain_place_failure(reason, request));
        }
        let result = receive
            .await
            .unwrap_or(Err(PmLoopbackMutationAuthError::AuthorityClosed));
        match result {
            Ok(retained) => Ok(retained),
            Err(reason) => Err(retain_place_failure(reason, request)),
        }
    }
}

impl fmt::Debug for PmLoopbackPlaceAuthenticationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmLoopbackPlaceAuthenticationRole(<opaque>)")
    }
}

pub struct PmLoopbackCancelAuthenticationRole {
    sender: mpsc::Sender<CancelAuthenticationRequest>,
}

impl PmLoopbackCancelAuthenticationRole {
    /// Authenticate one exact-owned cancel request.
    ///
    /// # Cancellation contract
    ///
    /// The supervised authenticated worker MUST await this future to
    /// completion without selecting it against cancellation. Rust cannot
    /// return a moved request to a caller that drops its future.
    /// Process/task stop recovers durable Goal-F intent; it must never resend.
    pub async fn authenticate_cancel(
        &mut self,
        request: PmExactOwnedCancelRequest,
        server_time: PmAuthorizedMutationServerTime,
    ) -> Result<PmRetainedOwnedCancelRequest, PmLoopbackCancelAuthenticationFailure> {
        let request = Arc::new(request);
        let timestamp = server_time.into_l2_timestamp();
        let (response, receive) = oneshot::channel();
        if let Err(error) = self.sender.try_send(CancelAuthenticationRequest {
            request: Arc::clone(&request),
            timestamp,
            response,
        }) {
            let reason = classify_cancel_send_error(error);
            return Err(retain_cancel_failure(reason, request));
        }
        let result = receive
            .await
            .unwrap_or(Err(PmLoopbackMutationAuthError::AuthorityClosed));
        match result {
            Ok(retained) => Ok(retained),
            Err(reason) => Err(retain_cancel_failure(reason, request)),
        }
    }
}

impl fmt::Debug for PmLoopbackCancelAuthenticationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmLoopbackCancelAuthenticationRole(<opaque>)")
    }
}

struct PlaceAuthenticationRequest {
    request: Arc<PmGtcPostOnlyPlaceRequest>,
    timestamp: L2Timestamp,
    response: oneshot::Sender<Result<PmRetainedPlaceRequest, PmLoopbackMutationAuthError>>,
}

struct CancelAuthenticationRequest {
    request: Arc<PmExactOwnedCancelRequest>,
    timestamp: L2Timestamp,
    response: oneshot::Sender<Result<PmRetainedOwnedCancelRequest, PmLoopbackMutationAuthError>>,
}

struct AuthoritySenders {
    read: mpsc::Sender<CredentialRequest>,
    place: mpsc::Sender<PlaceAuthenticationRequest>,
    cancel: mpsc::Sender<CancelAuthenticationRequest>,
    supervisor: PmCredentialAuthoritySupervisor,
}

fn spawn_loopback_authority(
    binding: MutationBinding,
    signer: FixedEoaSigner,
    credentials: L2Credentials,
) -> Result<AuthoritySenders, PmLoopbackMutationAuthError> {
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
        PmLoopbackMutationAuthError::InvalidConfiguration(
            "loopback mutation authority requires an active Tokio runtime",
        )
    })?;
    let (read, read_receiver) = mpsc::channel(MUTATION_AUTHORITY_CAPACITY);
    let (place, place_receiver) = mpsc::channel(MUTATION_AUTHORITY_CAPACITY);
    let (cancel, cancel_receiver) = mpsc::channel(MUTATION_AUTHORITY_CAPACITY);
    let (shutdown, shutdown_receiver) = oneshot::channel();
    let task = runtime.spawn(run_loopback_authority(
        binding,
        signer,
        credentials,
        read_receiver,
        place_receiver,
        cancel_receiver,
        shutdown_receiver,
    ));
    Ok(AuthoritySenders {
        read,
        place,
        cancel,
        supervisor: PmCredentialAuthoritySupervisor::from_task(shutdown, task),
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_loopback_authority(
    binding: MutationBinding,
    signer: FixedEoaSigner,
    credentials: L2Credentials,
    mut read: mpsc::Receiver<CredentialRequest>,
    mut place: mpsc::Receiver<PlaceAuthenticationRequest>,
    mut cancel: mpsc::Receiver<CancelAuthenticationRequest>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut read_open = true;
    let mut place_open = true;
    let mut cancel_open = true;
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                read.close();
                place.close();
                cancel.close();
                return;
            }
            request = cancel.recv(), if cancel_open => match request {
                Some(request) => respond_cancel(request, &binding, &credentials),
                None => cancel_open = false,
            },
            request = read.recv(), if read_open => match request {
                Some(request) => handle_credential_request(&credentials, request),
                None => read_open = false,
            },
            request = place.recv(), if place_open => match request {
                Some(request) => respond_place(request, &binding, &signer, &credentials),
                None => place_open = false,
            },
            else => return,
        }
    }
}

fn respond_place(
    request: PlaceAuthenticationRequest,
    binding: &MutationBinding,
    signer: &FixedEoaSigner,
    credentials: &L2Credentials,
) {
    let PlaceAuthenticationRequest {
        request,
        timestamp,
        response,
    } = request;
    let value = authenticate_place(binding, signer, credentials, request.as_ref(), timestamp);
    // Drop the authority task's shared borrow before waking the caller. This
    // guarantees an error path can recover sole ownership immediately.
    drop(request);
    let _ = response.send(value);
}

fn authenticate_place(
    binding: &MutationBinding,
    signer: &FixedEoaSigner,
    credentials: &L2Credentials,
    request: &PmGtcPostOnlyPlaceRequest,
    timestamp: L2Timestamp,
) -> Result<PmRetainedPlaceRequest, PmLoopbackMutationAuthError> {
    if request.account_scope() != binding.account
        || request.instrument() != binding.instrument
        || request.instrument_id() != binding.trading_domain.instrument()
        || request.trading_domain() != binding.trading_domain
        || request.client_order().account() != binding.account.handle()
    {
        return Err(PmLoopbackMutationAuthError::PlaceScopeMismatch);
    }
    let profile = request.profile();
    if profile != binding.place_profile
        || profile.order_type() != PmFixedOrderType::Gtc
        || !profile.post_only()
        || profile.defer_exec()
        || profile.expiration() != 0
    {
        return Err(PmLoopbackMutationAuthError::PlaceProfileMismatch);
    }
    let unsigned = request.unsigned_order();
    let expected = exact_order_amounts(request.side(), request.price(), request.quantity())
        .map_err(|_| PmLoopbackMutationAuthError::PlaceOrderMismatch)?;
    let eoa_signer = binding.account.signer().address();
    let expected_maker = binding.account.funder().address();
    let identity_matches = match binding.signature_profile {
        PmClobV2SignatureType::Eoa => {
            expected_maker == eoa_signer
                && unsigned.maker() == eoa_signer
                && unsigned.signer() == eoa_signer
                && unsigned.signature_profile() == PmClobV2SignatureType::Eoa
        }
        PmClobV2SignatureType::Proxy => {
            expected_maker != eoa_signer
                && unsigned.maker() == expected_maker
                && unsigned.signer() == eoa_signer
                && unsigned.signature_profile() == PmClobV2SignatureType::Proxy
        }
    };
    if !identity_matches
        || unsigned.token_id() != binding.trading_domain.instrument().token()
        || unsigned.side() != request.side()
        || unsigned.maker_amount() != expected.maker()
        || unsigned.taker_amount() != expected.taker()
    {
        return Err(PmLoopbackMutationAuthError::PlaceOrderMismatch);
    }
    let signed = signer.sign_clob_v2_order(binding.clob_domain, unsigned)?;
    let serialized = credentials.serialize_gtc_post_only(signed)?;
    let authenticated = credentials.authenticate_place(timestamp, serialized)?;
    Ok(PmRetainedPlaceRequest::retain(authenticated)?)
}

fn respond_cancel(
    request: CancelAuthenticationRequest,
    binding: &MutationBinding,
    credentials: &L2Credentials,
) {
    let CancelAuthenticationRequest {
        request,
        timestamp,
        response,
    } = request;
    let value = authenticate_cancel(binding, credentials, request.as_ref(), timestamp);
    drop(request);
    let _ = response.send(value);
}

fn authenticate_cancel(
    binding: &MutationBinding,
    credentials: &L2Credentials,
    request: &PmExactOwnedCancelRequest,
    timestamp: L2Timestamp,
) -> Result<PmRetainedOwnedCancelRequest, PmLoopbackMutationAuthError> {
    if request.account_scope() != binding.account
        || request.instrument() != binding.instrument
        || request.instrument_id() != binding.trading_domain.instrument()
        || request.client_order().account() != binding.account.handle()
        || request.venue_order().account() != binding.account.handle()
    {
        return Err(PmLoopbackMutationAuthError::CancelScopeMismatch);
    }
    if request.purpose() != binding.cancel_purpose {
        return Err(PmLoopbackMutationAuthError::CancelPurposeMismatch);
    }
    let order_id = FixedOrderId::parse(request.venue_order().id().as_str())?;
    let serialized = credentials.serialize_owned_cancel(order_id)?;
    let authenticated = credentials.authenticate_owned_cancel(timestamp, serialized)?;
    Ok(PmRetainedOwnedCancelRequest::retain(authenticated)?)
}

fn classify_place_send_error(
    error: mpsc::error::TrySendError<PlaceAuthenticationRequest>,
) -> PmLoopbackMutationAuthError {
    match error {
        mpsc::error::TrySendError::Full(message) => {
            drop(message);
            PmLoopbackMutationAuthError::AuthoritySaturated
        }
        mpsc::error::TrySendError::Closed(message) => {
            drop(message);
            PmLoopbackMutationAuthError::AuthorityClosed
        }
    }
}

fn classify_cancel_send_error(
    error: mpsc::error::TrySendError<CancelAuthenticationRequest>,
) -> PmLoopbackMutationAuthError {
    match error {
        mpsc::error::TrySendError::Full(message) => {
            drop(message);
            PmLoopbackMutationAuthError::AuthoritySaturated
        }
        mpsc::error::TrySendError::Closed(message) => {
            drop(message);
            PmLoopbackMutationAuthError::AuthorityClosed
        }
    }
}

fn retain_place_failure(
    reason: PmLoopbackMutationAuthError,
    request: Arc<PmGtcPostOnlyPlaceRequest>,
) -> PmLoopbackPlaceAuthenticationFailure {
    let request = Arc::try_unwrap(request)
        .unwrap_or_else(|_| unreachable!("place authority released its request before response"));
    PmLoopbackPlaceAuthenticationFailure { reason, request }
}

fn retain_cancel_failure(
    reason: PmLoopbackMutationAuthError,
    request: Arc<PmExactOwnedCancelRequest>,
) -> PmLoopbackCancelAuthenticationFailure {
    let request = Arc::try_unwrap(request)
        .unwrap_or_else(|_| unreachable!("cancel authority released its request before response"));
    PmLoopbackCancelAuthenticationFailure { reason, request }
}

#[cfg(test)]
mod tests;
