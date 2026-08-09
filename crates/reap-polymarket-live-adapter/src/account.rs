use reap_pm_core::{EvmAddress, PmTokenId, U256};
use reap_polymarket_wire::{PmLiveBalanceAllowance, parse_live_balance_allowance};

use crate::{
    PmLiveAdapterError, PmReadServerTime,
    private_credentials::PmHttpCredentialRole,
    private_http::{PmPrivateHttpObservation, PmPrivateHttpTransport, PmPrivateRoute},
};

/// Closed Polymarket account profile accepted by the read-only balance route.
///
/// This value affects only the `signature_type` query parameter on the two
/// fixed balance/allowance GETs. It does not construct signing or mutation
/// authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PmReadOnlySignatureType {
    /// The L2-authenticated EOA is also the account funder.
    Eoa = 0,
    /// The L2-authenticated EOA reads balances held by its Polymarket proxy.
    Proxy = 1,
}

impl PmReadOnlySignatureType {
    #[must_use]
    pub const fn value(self) -> u8 {
        self as u8
    }

    pub(crate) const fn query_value(self) -> &'static str {
        match self {
            Self::Eoa => "0",
            Self::Proxy => "1",
        }
    }
}

impl TryFrom<u8> for PmReadOnlySignatureType {
    type Error = PmLiveAdapterError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Eoa),
            1 => Ok(Self::Proxy),
            _ => Err(PmLiveAdapterError::InvalidConfiguration(
                "read-only balance signature_type must be 0 (EOA) or 1 (proxy)",
            )),
        }
    }
}

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
    signature_type: PmReadOnlySignatureType,
}

/// Move-only owner for exactly the two authenticated account GETs.
///
/// Unlike [`crate::PmAuthenticatedHttpOwner`], this type cannot construct
/// reconciliation or exact-order capabilities.
pub struct PmReadOnlyAccountHttpOwner {
    authority: PmHttpCredentialRole,
    transport: PmPrivateHttpTransport,
    conditional_token: PmTokenId,
    signature_type: PmReadOnlySignatureType,
}

impl PmReadOnlyAccountHttpOwner {
    pub(crate) const fn from_authority(
        authority: PmHttpCredentialRole,
        transport: PmPrivateHttpTransport,
        conditional_token: PmTokenId,
        signature_type: PmReadOnlySignatureType,
    ) -> Self {
        Self {
            authority,
            transport,
            conditional_token,
            signature_type,
        }
    }

    pub fn account(&mut self) -> PmAccountHttpRole<'_> {
        PmAccountHttpRole::new(
            &mut self.authority,
            &self.transport,
            self.conditional_token,
            self.signature_type,
        )
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl std::fmt::Debug for PmReadOnlyAccountHttpOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PmReadOnlyAccountHttpOwner([REDACTED])")
    }
}

impl<'a> PmAccountHttpRole<'a> {
    pub(crate) const fn new(
        authority: &'a mut PmHttpCredentialRole,
        transport: &'a PmPrivateHttpTransport,
        conditional_token: PmTokenId,
        signature_type: PmReadOnlySignatureType,
    ) -> Self {
        Self {
            authority,
            transport,
            conditional_token,
            signature_type,
        }
    }

    pub async fn collateral_balance_allowance(
        &mut self,
        server_time: PmReadServerTime,
    ) -> Result<PmAccountBalanceAllowance, PmLiveAdapterError> {
        self.read(
            server_time,
            PmPrivateRoute::CollateralBalanceAllowance(self.signature_type),
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
            PmPrivateRoute::ConditionalBalanceAllowance {
                token: self.conditional_token,
                signature_type: self.signature_type,
            },
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
