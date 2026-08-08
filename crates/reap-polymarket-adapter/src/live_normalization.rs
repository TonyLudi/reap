mod order;
mod trade;

use reap_pm_core::{EvmAddress, PmAccountScope, PmProductSource, PmTokenId};
use thiserror::Error;

use crate::{PmFillCutError, PmFixtureInstrumentScope, PmForeignDiagnosticError};

pub(crate) use order::{normalize_rest_order, normalize_user_order};
pub(crate) use trade::{fill_event_from_leg, normalize_trade};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmLiveNormalizationError {
    #[error("live PM account profile requires signer, funder, and observed maker to be one EOA")]
    AccountProfileMismatch,
    #[error("live PM configured order explicitly names an unsupported order type")]
    UnsupportedOrderType,
    #[error("live PM configured GTC order carries nonzero expiration")]
    UnexpectedExpiration,
    #[error("live PM configured order outcome contradicts configured metadata")]
    OutcomeMismatch,
    #[error("live PM configured order or fill price is off the configured tick")]
    PriceOffTick,
    #[error("live PM configured original order quantity violates the fixed order contract")]
    InvalidOrderQuantityContract,
    #[error("live PM price and quantity do not produce integral protocol amounts")]
    NonIntegralProtocolAmounts,
    #[error("live PM order progress contradicts its status or original quantity")]
    InvalidOrderProgress,
    #[error("live PM order status is outside the accepted exact status set")]
    UnknownOrderStatus,
    #[error("live PM open-order cut contains a terminal configured order")]
    OpenOrderIsTerminal,
    #[error("live PM user-order event kind is outside the accepted minimal schema")]
    UnknownUserOrderKind,
    #[error("live PM user-order event kind contradicts its cumulative progress")]
    UserOrderKindProgressMismatch,
    #[error("live PM user-order wire status contradicts its event kind or cumulative progress")]
    UserOrderStatusProgressMismatch,
    #[error("live PM configured user order omitted required profile fact `{0}`")]
    MissingUserOrderProfileFact(&'static str),
    #[error("live PM trade settlement status is outside the accepted exact status set")]
    UnknownTradeStatus,
    #[error("one exact live PM order identity carries conflicting facts")]
    ConflictingOrder,
    #[error("one exact live PM trade-leg identity carries conflicting facts")]
    ConflictingTradeLeg,
    #[error("live PM complete cut is empty or lacks a terminal page")]
    IncompleteCut,
    #[error("live PM complete cut contains rows after an earlier terminal page")]
    PageAfterTerminal,
    #[error("live PM complete cut exceeds its fixed normalized-row bound")]
    TooManyRows,
    #[error("complete live PM reconciliation cannot prove an exact local trade leg")]
    UnresolvedCompleteTrade,
    #[error("live PM account response omits the exact required spender allowance")]
    MissingRequiredAllowance,
    #[error("normalized live PM event violates the core event contract")]
    EventContract,
    #[error("live PM foreign-row diagnostic contract failed: {0}")]
    ForeignDiagnostic(#[from] PmForeignDiagnosticError),
    #[error("live PM canonical fill-cut contract failed: {0}")]
    FillCut(#[from] PmFillCutError),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LiveNormalizationScope {
    pub(crate) account: PmAccountScope,
    pub(crate) instrument: PmFixtureInstrumentScope,
    pub(crate) source: PmProductSource,
}

impl LiveNormalizationScope {
    pub(crate) fn validate_account_profile(self) -> Result<EvmAddress, PmLiveNormalizationError> {
        let signer = self.account.signer().address();
        let funder = self.account.funder().address();
        if signer != funder {
            return Err(PmLiveNormalizationError::AccountProfileMismatch);
        }
        Ok(funder)
    }

    pub(crate) fn is_configured(
        self,
        condition: reap_pm_core::PmConditionId,
        token: PmTokenId,
    ) -> bool {
        condition == self.instrument.metadata().condition()
            && token == self.instrument.metadata().outcome().token()
    }
}
