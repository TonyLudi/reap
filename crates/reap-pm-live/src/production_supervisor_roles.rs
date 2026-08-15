//! Concrete fixed production edge authorities for the supervisor actor.

use std::{collections::BTreeMap, fmt};

use async_trait::async_trait;
use reap_pm_core::EvmAddress;
use reap_polymarket_auth::L2Credentials;
use reap_polymarket_live_adapter::{
    PmMutationClassification, PmOrderHeartbeatProductionRole, PmOrderHeartbeatReply,
    PmProductionPostOnlyPlaceRequest, PmProductionSupervisedMutationRole, PmReadServerTimeHttpRole,
};
use reap_polymarket_wire::PmOrderHeartbeatId;

use crate::production_supervisor::{
    MAX_PM_SUPERVISOR_TOKENS, PmProductionSupervisorError, PmSupervisorCancelResult,
    PmSupervisorEdgeError, PmSupervisorHeartbeatRole, PmSupervisorMutationClassification,
    PmSupervisorMutationRole, PmSupervisorOrderFacts, PmSupervisorPlaceResult, PmSupervisorScope,
};

/// Concrete one-tick heartbeat role. Scheduling and fatal supervision remain
/// with [`crate::PmProductionSupervisor`]; this value retains the
/// credential-wide identifier and exact fixed `/time` + `/v1/heartbeats`
/// capabilities.
pub struct PmSupervisorFixedHeartbeatRole {
    credentials: L2Credentials,
    server_time: PmReadServerTimeHttpRole,
    transport: PmOrderHeartbeatProductionRole,
    previous: Option<PmOrderHeartbeatId>,
}

impl PmSupervisorFixedHeartbeatRole {
    pub fn new(
        credentials: L2Credentials,
        server_time: PmReadServerTimeHttpRole,
        transport: PmOrderHeartbeatProductionRole,
    ) -> Result<Self, PmProductionSupervisorError> {
        if !server_time.is_production() {
            return Err(PmProductionSupervisorError::InvalidConfiguration);
        }
        Ok(Self {
            credentials,
            server_time,
            transport,
            previous: None,
        })
    }

    pub(crate) fn configured_l2_signer(&self) -> EvmAddress {
        self.credentials.address().as_core()
    }

    async fn authenticate_and_send(
        &mut self,
        previous: Option<PmOrderHeartbeatId>,
    ) -> Result<PmOrderHeartbeatReply, PmSupervisorEdgeError> {
        let timestamp = self
            .server_time
            .fresh_read_server_time_observation()
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?
            .parsed_l2_timestamp();
        let request = match previous.as_ref() {
            Some(previous) => self
                .credentials
                .authenticate_order_heartbeat(timestamp, previous),
            None => self
                .credentials
                .authenticate_initial_order_heartbeat(timestamp),
        }
        .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        self.transport
            .send(request)
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)
    }
}

impl fmt::Debug for PmSupervisorFixedHeartbeatRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmSupervisorFixedHeartbeatRole([REDACTED; FIXED PRODUCTION])")
    }
}

#[async_trait]
impl PmSupervisorHeartbeatRole for PmSupervisorFixedHeartbeatRole {
    async fn heartbeat(&mut self) -> Result<(), PmSupervisorEdgeError> {
        let previous = self.previous.take();
        match self.authenticate_and_send(previous).await? {
            PmOrderHeartbeatReply::Accepted(next) => self.previous = Some(next),
            PmOrderHeartbeatReply::StaleIdentifier(current) => {
                match self.authenticate_and_send(Some(current)).await? {
                    PmOrderHeartbeatReply::Accepted(next) => self.previous = Some(next),
                    PmOrderHeartbeatReply::StaleIdentifier(_) => {
                        return Err(PmSupervisorEdgeError::InvalidObservation);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Concrete adapter from the reviewed fixed-scope signer/L2/time/transport
/// owner to the generic supervisor actor.
pub struct PmSupervisorFixedMutationRole {
    inner: PmProductionSupervisedMutationRole,
}

impl PmSupervisorFixedMutationRole {
    #[must_use]
    pub const fn new(inner: PmProductionSupervisedMutationRole) -> Self {
        Self { inner }
    }
}

impl fmt::Debug for PmSupervisorFixedMutationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmSupervisorFixedMutationRole(<opaque production authority>)")
    }
}

#[async_trait]
impl PmSupervisorMutationRole for PmSupervisorFixedMutationRole {
    type PlaceRequest = PmProductionPostOnlyPlaceRequest;

    fn validate_place(&self, facts: &PmSupervisorOrderFacts, request: &Self::PlaceRequest) -> bool {
        let order = request.order();
        self.inner.validate_place(request)
            && self
                .inner
                .expected_order_id(request)
                .is_some_and(|order_id| order_id.to_string() == facts.expected_venue_order_id())
            && order.token_id().units().to_string() == facts.token_id()
            && order.side() == facts.side()
            && request.quantity().protocol_units() == facts.quantity()
    }

    async fn place(
        &mut self,
        request: Self::PlaceRequest,
    ) -> Result<PmSupervisorPlaceResult, PmSupervisorEdgeError> {
        let outcome = self
            .inner
            .place(request)
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        Ok(PmSupervisorPlaceResult {
            classification: map_mutation_classification(outcome.classification()),
            observed_venue_order_id: outcome
                .observed_order_id()
                .map(|order_id| order_id.as_str().to_owned()),
        })
    }

    async fn cancel_exact(
        &mut self,
        venue_order_id: &str,
        token_id: &str,
    ) -> Result<PmSupervisorCancelResult, PmSupervisorEdgeError> {
        if self.inner.configured_scope().token().units().to_string() != token_id {
            return Err(PmSupervisorEdgeError::InvalidObservation);
        }
        let outcome = self
            .inner
            .cancel_exact(venue_order_id)
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        Ok(PmSupervisorCancelResult {
            classification: map_mutation_classification(outcome.classification()),
        })
    }
}

/// Concrete multi-token mutation router for one condition/market. Each token
/// retains an independent fixed-scope signer/time/transport owner; exact
/// cancellation is routed using durable order facts rather than an in-memory
/// placement registry, so recovery remains deterministic.
pub struct PmSupervisorProductionMutationRole {
    condition_id: String,
    l2_signer: EvmAddress,
    expected_maker: EvmAddress,
    roles: BTreeMap<String, PmProductionSupervisedMutationRole>,
}

impl PmSupervisorProductionMutationRole {
    pub fn new(
        roles: impl IntoIterator<Item = PmProductionSupervisedMutationRole>,
    ) -> Result<Self, PmProductionSupervisorError> {
        let mut condition = None;
        let mut market = None;
        let mut l2_signer = None;
        let mut expected_maker = None;
        let mut by_token = BTreeMap::new();
        for role in roles {
            let scope = role.configured_scope();
            if condition.is_some_and(|expected| expected != scope.condition())
                || market.is_some_and(|expected| expected != scope.market())
                || l2_signer.is_some_and(|expected| expected != role.configured_l2_signer())
                || expected_maker
                    .is_some_and(|expected| expected != role.configured_expected_maker())
            {
                return Err(PmProductionSupervisorError::InvalidConfiguration);
            }
            condition = Some(scope.condition());
            market = Some(scope.market());
            l2_signer = Some(role.configured_l2_signer());
            expected_maker = Some(role.configured_expected_maker());
            if by_token
                .insert(scope.token().units().to_string(), role)
                .is_some()
            {
                return Err(PmProductionSupervisorError::InvalidConfiguration);
            }
        }
        if by_token.is_empty() || by_token.len() > MAX_PM_SUPERVISOR_TOKENS {
            return Err(PmProductionSupervisorError::InvalidConfiguration);
        }
        Ok(Self {
            condition_id: condition
                .expect("nonempty production mutation role set")
                .to_string(),
            l2_signer: l2_signer.expect("nonempty production mutation role set"),
            expected_maker: expected_maker.expect("nonempty production mutation role set"),
            roles: by_token,
        })
    }

    pub(crate) fn matches_supervisor_scope(
        &self,
        scope: &PmSupervisorScope,
        l2_signer: EvmAddress,
        expected_maker: EvmAddress,
    ) -> bool {
        self.condition_id == scope.condition_id()
            && self.l2_signer == l2_signer
            && self.expected_maker == expected_maker
            && self
                .roles
                .keys()
                .map(String::as_str)
                .eq(scope.token_ids().iter().map(String::as_str))
    }
}

impl fmt::Debug for PmSupervisorProductionMutationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmSupervisorProductionMutationRole")
            .field("token_roles", &self.roles.len())
            .field("authority", &"<opaque production authorities>")
            .finish()
    }
}

#[async_trait]
impl PmSupervisorMutationRole for PmSupervisorProductionMutationRole {
    type PlaceRequest = PmProductionPostOnlyPlaceRequest;

    fn validate_place(&self, facts: &PmSupervisorOrderFacts, request: &Self::PlaceRequest) -> bool {
        let token_id = request.order().token_id().units().to_string();
        token_id == facts.token_id()
            && self.roles.get(&token_id).is_some_and(|role| {
                let order = request.order();
                role.validate_place(request)
                    && role.expected_order_id(request).is_some_and(|order_id| {
                        order_id.to_string() == facts.expected_venue_order_id()
                    })
                    && order.side() == facts.side()
                    && request.quantity().protocol_units() == facts.quantity()
            })
    }

    async fn place(
        &mut self,
        request: Self::PlaceRequest,
    ) -> Result<PmSupervisorPlaceResult, PmSupervisorEdgeError> {
        let token_id = request.order().token_id().units().to_string();
        let role = self
            .roles
            .get_mut(&token_id)
            .ok_or(PmSupervisorEdgeError::InvalidObservation)?;
        let outcome = role
            .place(request)
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        Ok(PmSupervisorPlaceResult {
            classification: map_mutation_classification(outcome.classification()),
            observed_venue_order_id: outcome
                .observed_order_id()
                .map(|order_id| order_id.as_str().to_owned()),
        })
    }

    async fn cancel_exact(
        &mut self,
        venue_order_id: &str,
        token_id: &str,
    ) -> Result<PmSupervisorCancelResult, PmSupervisorEdgeError> {
        let role = self
            .roles
            .get_mut(token_id)
            .ok_or(PmSupervisorEdgeError::InvalidObservation)?;
        let outcome = role
            .cancel_exact(venue_order_id)
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        Ok(PmSupervisorCancelResult {
            classification: map_mutation_classification(outcome.classification()),
        })
    }
}

const fn map_mutation_classification(
    classification: PmMutationClassification,
) -> PmSupervisorMutationClassification {
    match classification {
        PmMutationClassification::DefinitelyNotDispatched => {
            PmSupervisorMutationClassification::DefinitelyNotDispatched
        }
        PmMutationClassification::Accepted => PmSupervisorMutationClassification::Accepted,
        PmMutationClassification::Rejected => PmSupervisorMutationClassification::Rejected,
        PmMutationClassification::OutOfProfile
        | PmMutationClassification::AcknowledgementUnknown => {
            PmSupervisorMutationClassification::AcknowledgementUnknown
        }
    }
}
