//! Purpose-closed production mutation owner for the continuous supervisor.
//!
//! This is the only default-build seam that joins the selected, scope-bound
//! place/cancel clock capsule with a fixed signer/L2 proxy profile and the
//! reviewed fixed-peer transports. It accepts no origin, generic request,
//! alternate route, batch, market cancel, or cancel-all capability.

use std::fmt;

use reap_pm_core::{EvmAddress, PmOrderSide, PmQuantity};
use reap_polymarket_auth::{
    FixedEoaSigner, FixedOrderId, L2Credentials, PmClobDomain, derive_place_public_request_identity,
};
use reap_polymarket_wire::{PmClobV2SignatureType, PmUnsignedClobV2Order, PmWireScope};
use thiserror::Error;

use crate::{
    PmCancelMutationOutcome, PmCancelMutationTimeFinalizer, PmCancelServerTimeHttpRole,
    PmExactOwnedCancelProductionRole, PmFixedPlaceProductionRole, PmMutationEdgeError,
    PmPlaceMutationOutcome, PmPlaceMutationTimeFinalizer, PmPlaceServerTimeHttpRole,
    PmProductionMutationConfig, PmProductionSelectedPlaceCancelTimeOwner,
    PmRetainedOwnedCancelRequest, PmRetainedPlaceRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmProductionSupervisedMutationError {
    #[error("production supervised mutation configuration is invalid")]
    InvalidConfiguration,
    #[error("production supervised place request is outside the fixed scope or profile")]
    PlaceScopeMismatch,
    #[error("production supervised exact-owned cancel identity is invalid")]
    InvalidCancelIdentity,
    #[error("production supervised mutation server-time request failed")]
    ServerTime,
    #[error("production supervised mutation signing or authentication failed")]
    Authentication,
    #[error("production supervised mutation transport construction failed")]
    Transport,
}

impl From<PmMutationEdgeError> for PmProductionSupervisedMutationError {
    fn from(_: PmMutationEdgeError) -> Self {
        Self::Transport
    }
}

/// Move-only, unsigned public order facts admitted by the production
/// supervisor. The signer and L2 credentials remain inside the role.
pub struct PmProductionPostOnlyPlaceRequest {
    order: PmUnsignedClobV2Order,
    quantity: PmQuantity,
}

impl PmProductionPostOnlyPlaceRequest {
    #[must_use]
    pub const fn new(order: PmUnsignedClobV2Order, quantity: PmQuantity) -> Self {
        Self { order, quantity }
    }

    #[must_use]
    pub const fn order(&self) -> PmUnsignedClobV2Order {
        self.order
    }

    #[must_use]
    pub const fn quantity(&self) -> PmQuantity {
        self.quantity
    }
}

impl fmt::Debug for PmProductionPostOnlyPlaceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmProductionPostOnlyPlaceRequest")
            .field("order", &self.order)
            .field("quantity", &self.quantity)
            .finish()
    }
}

/// Sole signer/L2/time/transport owner used by a continuous supervisor actor.
pub struct PmProductionSupervisedMutationRole {
    scope: PmWireScope,
    domain: PmClobDomain,
    expected_proxy_maker: EvmAddress,
    signer: FixedEoaSigner,
    credentials: L2Credentials,
    place_time: PmPlaceServerTimeHttpRole,
    place_finalizer: PmPlaceMutationTimeFinalizer,
    cancel_time: PmCancelServerTimeHttpRole,
    cancel_finalizer: PmCancelMutationTimeFinalizer,
    place_transport: PmFixedPlaceProductionRole,
    cancel_transport: PmExactOwnedCancelProductionRole,
}

impl PmProductionSupervisedMutationRole {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        time_owner: PmProductionSelectedPlaceCancelTimeOwner,
        domain: PmClobDomain,
        expected_proxy_maker: EvmAddress,
        signer: FixedEoaSigner,
        credentials: L2Credentials,
        transport_config: PmProductionMutationConfig,
    ) -> Result<Self, PmProductionSupervisedMutationError> {
        let signer_address = signer.address().as_core();
        if credentials.address().as_core() != signer_address
            || expected_proxy_maker == signer_address
            || expected_proxy_maker.bytes() == [0; 20]
        {
            return Err(PmProductionSupervisedMutationError::InvalidConfiguration);
        }
        let (place_time_owner, cancel_time_owner, scope) =
            time_owner.into_supervised_purpose_owners();
        let (place_time, place_finalizer) = place_time_owner.into_roles();
        let (cancel_time, cancel_finalizer) = cancel_time_owner.into_roles();
        let place_transport = PmFixedPlaceProductionRole::new(transport_config.clone())?;
        let cancel_transport = PmExactOwnedCancelProductionRole::new(transport_config)?;
        Ok(Self {
            scope,
            domain,
            expected_proxy_maker,
            signer,
            credentials,
            place_time,
            place_finalizer,
            cancel_time,
            cancel_finalizer,
            place_transport,
            cancel_transport,
        })
    }

    #[must_use]
    pub fn validate_place(&self, request: &PmProductionPostOnlyPlaceRequest) -> bool {
        let order = request.order;
        let quantity_units = match order.side() {
            PmOrderSide::Buy => order.taker_amount(),
            PmOrderSide::Sell => order.maker_amount(),
        };
        order.signature_profile() == PmClobV2SignatureType::Proxy
            && order.maker() == self.expected_proxy_maker
            && order.signer() == self.signer.address().as_core()
            && order.signer() == self.credentials.address().as_core()
            && order.token_id() == self.scope.token()
            && quantity_units == request.quantity.protocol_units()
    }

    #[must_use]
    pub fn expected_order_id(
        &self,
        request: &PmProductionPostOnlyPlaceRequest,
    ) -> Option<FixedOrderId> {
        self.validate_place(request).then(|| {
            derive_place_public_request_identity(self.domain, request.order)
                .expected_order_id()
                .into()
        })
    }

    pub async fn place(
        &mut self,
        request: PmProductionPostOnlyPlaceRequest,
    ) -> Result<PmPlaceMutationOutcome, PmProductionSupervisedMutationError> {
        if !self.validate_place(&request) {
            return Err(PmProductionSupervisedMutationError::PlaceScopeMismatch);
        }
        let signed = self
            .signer
            .sign_clob_v2_order(self.domain, request.order)
            .map_err(|_| PmProductionSupervisedMutationError::Authentication)?;
        let serialized = self
            .credentials
            .serialize_gtc_post_only(signed)
            .map_err(|_| PmProductionSupervisedMutationError::Authentication)?;
        let observation = self
            .place_time
            .fresh_place_time_observation()
            .await
            .map_err(|_| PmProductionSupervisedMutationError::ServerTime)?;
        let seconds = observation.observed_l2_timestamp_seconds();
        let authenticated = self
            .place_finalizer
            .authenticate_exact_place(
                observation.into_proof(),
                seconds,
                &self.credentials,
                serialized,
            )
            .map_err(|_| PmProductionSupervisedMutationError::Authentication)?;
        let retained = PmRetainedPlaceRequest::retain(authenticated)
            .map_err(|_| PmProductionSupervisedMutationError::Authentication)?;
        Ok(self.place_transport.send(retained).await)
    }

    pub async fn cancel_exact(
        &mut self,
        venue_order_id: &str,
    ) -> Result<PmCancelMutationOutcome, PmProductionSupervisedMutationError> {
        let order_id = FixedOrderId::parse(venue_order_id)
            .map_err(|_| PmProductionSupervisedMutationError::InvalidCancelIdentity)?;
        let serialized = self
            .credentials
            .serialize_owned_cancel(order_id)
            .map_err(|_| PmProductionSupervisedMutationError::Authentication)?;
        let observation = self
            .cancel_time
            .fresh_cancel_time_observation()
            .await
            .map_err(|_| PmProductionSupervisedMutationError::ServerTime)?;
        let seconds = observation.observed_l2_timestamp_seconds();
        let authenticated = self
            .cancel_finalizer
            .authenticate_exact_owned_cancel(
                observation.into_proof(),
                seconds,
                &self.credentials,
                serialized,
            )
            .map_err(|_| PmProductionSupervisedMutationError::Authentication)?;
        let retained = PmRetainedOwnedCancelRequest::retain(authenticated)
            .map_err(|_| PmProductionSupervisedMutationError::Authentication)?;
        Ok(self.cancel_transport.send(retained).await)
    }

    #[must_use]
    pub const fn configured_scope(&self) -> PmWireScope {
        self.scope
    }
}

impl fmt::Debug for PmProductionSupervisedMutationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmProductionSupervisedMutationRole(<fixed scope/domain/signer/L2/time/transports; redacted>)",
        )
    }
}
