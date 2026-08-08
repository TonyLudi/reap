use reap_pm_core::{EvmAddress, PmTokenId, U256};
use reap_polymarket_wire::{PmLiveBalanceAllowance, parse_live_balance_allowance};

use crate::{
    PmLiveAdapterError, PmReadServerTime,
    private_credentials::PmHttpCredentialRole,
    private_http::{PmPrivateHttpObservation, PmPrivateHttpTransport, PmPrivateRoute},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmAccountAsset {
    Collateral,
    Conditional(PmTokenId),
}

/// A parsed balance/allowance observation bound to the exact requested asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmAccountBalanceAllowance {
    asset: PmAccountAsset,
    value: PmLiveBalanceAllowance,
}

impl PmAccountBalanceAllowance {
    #[must_use]
    pub const fn asset(&self) -> PmAccountAsset {
        self.asset
    }

    #[must_use]
    pub const fn value(&self) -> &PmLiveBalanceAllowance {
        &self.value
    }

    #[must_use]
    pub fn exact_allowance(&self, spender: EvmAddress) -> Option<U256> {
        self.value.exact_allowance(spender)
    }

    #[must_use]
    pub fn into_value(self) -> PmLiveBalanceAllowance {
        self.value
    }

    /// Construct exact typed evidence for downstream contract tests.
    /// Production transport authority never enables this feature.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub const fn test_support_new(asset: PmAccountAsset, value: PmLiveBalanceAllowance) -> Self {
        Self { asset, value }
    }
}

/// Borrowed authenticated capability for exactly two read-only EOA account
/// projections: collateral and one configured conditional token.
pub struct PmAccountHttpRole<'a> {
    authority: &'a mut PmHttpCredentialRole,
    transport: &'a PmPrivateHttpTransport,
    conditional_token: PmTokenId,
}

impl<'a> PmAccountHttpRole<'a> {
    pub(crate) const fn new(
        authority: &'a mut PmHttpCredentialRole,
        transport: &'a PmPrivateHttpTransport,
        conditional_token: PmTokenId,
    ) -> Self {
        Self {
            authority,
            transport,
            conditional_token,
        }
    }

    pub async fn collateral_balance_allowance(
        &mut self,
        server_time: PmReadServerTime,
    ) -> Result<PmAccountBalanceAllowance, PmLiveAdapterError> {
        self.read(
            server_time,
            PmPrivateRoute::CollateralBalanceAllowance,
            PmAccountAsset::Collateral,
        )
        .await
    }

    pub async fn conditional_balance_allowance(
        &mut self,
        server_time: PmReadServerTime,
    ) -> Result<PmAccountBalanceAllowance, PmLiveAdapterError> {
        self.read(
            server_time,
            PmPrivateRoute::ConditionalBalanceAllowance(self.conditional_token),
            PmAccountAsset::Conditional(self.conditional_token),
        )
        .await
    }

    async fn read(
        &mut self,
        server_time: PmReadServerTime,
        route: PmPrivateRoute<'_>,
        asset: PmAccountAsset,
    ) -> Result<PmAccountBalanceAllowance, PmLiveAdapterError> {
        let headers = self
            .authority
            .authenticate_balance(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
            )
            .await?;
        let body = match self.transport.get(route, headers).await? {
            PmPrivateHttpObservation::Found(body) => body,
            PmPrivateHttpObservation::NotFound => {
                return Err(PmLiveAdapterError::UnexpectedStatus { status: 404 });
            }
        };
        let value = parse_live_balance_allowance(&body)?;
        Ok(PmAccountBalanceAllowance { asset, value })
    }
}
