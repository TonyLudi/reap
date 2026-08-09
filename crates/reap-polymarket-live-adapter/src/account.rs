use reap_pm_core::{EvmAddress, PmTokenId, U256};
use reap_polymarket_wire::{PmLiveBalanceAllowance, parse_live_balance_allowance};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    PM_CLOB_PRODUCTION_ORIGIN, PmLiveAdapterError, PmPrivateReadEdgeClock,
    PmPrivateReadProductClock, PmReadServerTime,
    config::OriginMode,
    private_credentials::PmHttpCredentialRole,
    private_http::{PmPrivateHttpObservation, PmPrivateHttpTransport, PmPrivateRoute},
    read_authority::PmHttpReadAuthorityProvider,
};

const BALANCE_ALLOWANCE_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.balance-allowance-observation.v1\0";

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

/// SHA-256 commitment to one fixed authenticated balance/allowance read.
/// Construction is private to the source role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmAccountBalanceAllowanceObservationCommitment([u8; 32]);

impl PmAccountBalanceAllowanceObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Sealed authenticated observation for one exact account asset.
///
/// The commitment binds the signer that authenticated the request. For a
/// signature-type-1 profile, no field claims that the configured proxy funder
/// was echoed or remotely attested by this response.
#[derive(Debug)]
pub struct PmAccountBalanceAllowanceObservation {
    balance_allowance: PmAccountBalanceAllowance,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmAccountBalanceAllowanceObservationCommitment,
}

impl PmAccountBalanceAllowanceObservation {
    fn from_source(
        balance_allowance: PmAccountBalanceAllowance,
        receive_clock: PmPrivateReadEdgeClock,
        commitment: PmAccountBalanceAllowanceObservationCommitment,
    ) -> Self {
        Self {
            balance_allowance,
            receive_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn balance_allowance(&self) -> &PmAccountBalanceAllowance {
        &self.balance_allowance
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmAccountBalanceAllowanceObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub fn into_balance_allowance(self) -> PmAccountBalanceAllowance {
        self.balance_allowance
    }
}

struct FetchedBalanceAllowance {
    raw_response: Zeroizing<Vec<u8>>,
    parsed: PmAccountBalanceAllowance,
}

/// Borrowed authenticated capability for exactly two read-only EOA account
/// projections: collateral and one configured conditional token.
pub struct PmAccountHttpRole<'a> {
    authority: &'a mut dyn PmHttpReadAuthorityProvider,
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
        authority: &'a mut dyn PmHttpReadAuthorityProvider,
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

    pub async fn collateral_balance_allowance_observation(
        &mut self,
        server_time: PmReadServerTime,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmAccountBalanceAllowanceObservation, PmLiveAdapterError> {
        self.read_observation(
            server_time,
            PmPrivateRoute::CollateralBalanceAllowance(self.signature_type),
            PmAccountAsset::Collateral,
            clock,
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

    pub async fn conditional_balance_allowance_observation(
        &mut self,
        server_time: PmReadServerTime,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmAccountBalanceAllowanceObservation, PmLiveAdapterError> {
        self.read_observation(
            server_time,
            PmPrivateRoute::ConditionalBalanceAllowance {
                token: self.conditional_token,
                signature_type: self.signature_type,
            },
            PmAccountAsset::Conditional(self.conditional_token),
            clock,
        )
        .await
    }

    async fn read(
        &mut self,
        server_time: PmReadServerTime,
        route: PmPrivateRoute<'_>,
        asset: PmAccountAsset,
    ) -> Result<PmAccountBalanceAllowance, PmLiveAdapterError> {
        Ok(self.read_source(server_time, route, asset).await?.parsed)
    }

    async fn read_observation(
        &mut self,
        server_time: PmReadServerTime,
        route: PmPrivateRoute<'_>,
        asset: PmAccountAsset,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmAccountBalanceAllowanceObservation, PmLiveAdapterError> {
        let fetched = self.read_source(server_time, route, asset).await?;
        // Sample only after the bounded authenticated body has completed and
        // strict parsing has succeeded. The source role, never its caller,
        // chooses this edge.
        let receive_clock = clock
            .observe_authenticated_read_complete()
            .map_err(|_| PmLiveAdapterError::ProductClock)?;
        let commitment = balance_allowance_observation_commitment(
            self.transport.mode(),
            self.signature_type,
            self.transport.configured_signer().bytes(),
            &fetched.raw_response,
            &fetched.parsed,
            receive_clock,
        );
        Ok(PmAccountBalanceAllowanceObservation::from_source(
            fetched.parsed,
            receive_clock,
            commitment,
        ))
    }

    async fn read_source(
        &mut self,
        server_time: PmReadServerTime,
        route: PmPrivateRoute<'_>,
        asset: PmAccountAsset,
    ) -> Result<FetchedBalanceAllowance, PmLiveAdapterError> {
        let headers = self
            .authority
            .authenticate_balance_allowance(
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
        Ok(FetchedBalanceAllowance {
            raw_response: body,
            parsed: PmAccountBalanceAllowance { asset, value },
        })
    }
}

pub(crate) fn balance_allowance_observation_commitment(
    mode: OriginMode,
    signature_type: PmReadOnlySignatureType,
    authenticated_signer: [u8; 20],
    raw_response: &[u8],
    parsed: &PmAccountBalanceAllowance,
    receive_clock: PmPrivateReadEdgeClock,
) -> PmAccountBalanceAllowanceObservationCommitment {
    let mut digest = Sha256::new();
    encode_balance_bytes(&mut digest, BALANCE_ALLOWANCE_OBSERVATION_COMMITMENT_DOMAIN);
    encode_balance_bytes(&mut digest, origin_mode_name(mode));
    encode_balance_bytes(&mut digest, PM_CLOB_PRODUCTION_ORIGIN.as_bytes());
    encode_balance_bytes(&mut digest, b"GET");
    encode_balance_bytes(&mut digest, b"/balance-allowance");
    digest.update([signature_type.value()]);
    digest.update(authenticated_signer);
    match parsed.asset() {
        PmAccountAsset::Collateral => digest.update([0]),
        PmAccountAsset::Conditional(token) => {
            digest.update([1]);
            digest.update(token.units().to_be_bytes());
        }
    }
    encode_balance_bytes(&mut digest, raw_response);
    digest.update(parsed.value().balance().to_be_bytes());
    digest.update(
        u32::try_from(parsed.value().allowances().len())
            .expect("bounded allowance count fits u32")
            .to_be_bytes(),
    );
    for allowance in parsed.value().allowances() {
        digest.update(allowance.spender().bytes());
        digest.update(allowance.amount().to_be_bytes());
    }
    digest.update([u8::from(parsed.value().unscoped_scalar_present())]);
    digest.update(receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(receive_clock.monotonic_receive_ns().to_be_bytes());
    PmAccountBalanceAllowanceObservationCommitment::from_source_bytes(digest.finalize().into())
}

fn encode_balance_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded balance commitment field length fits u64")
            .to_be_bytes(),
    );
    digest.update(value);
}

const fn origin_mode_name(mode: OriginMode) -> &'static [u8] {
    match mode {
        OriginMode::Production => b"production",
        #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
        OriginMode::LocalEvidence => b"local-evidence",
    }
}
