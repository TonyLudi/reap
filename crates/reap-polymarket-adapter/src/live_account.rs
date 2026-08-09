use reap_pm_core::{
    PmAllowanceValue, PmAssetId, PmErc1155OperatorApproval, PmPositionAvailability, PmSpenderId,
    PmSpenderRequirement,
};
use reap_polymarket_wire::PmLiveBalanceAllowance;
use sha2::Digest;

use crate::live_diagnostics::{ForeignDiagnosticsBuilder, semantic_hash};
use crate::live_normalization::{LiveNormalizationScope, PmLiveNormalizationError};
use crate::{
    PmCompleteAccountSnapshotDelivery, PmFixtureAllowanceRow, PmFixtureBalanceRow,
    PmFixturePositionRow, PmForeignRowDiagnostics,
};

const ACCOUNT_FOREIGN_DOMAIN: &[u8] = b"reap.pm.live.account-foreign.v1\0";
const ACCOUNT_ALLOWANCE_KEY_DOMAIN: &[u8] = b"reap.pm.live.account-allowance-key.v1\0";
const ACCOUNT_ALLOWANCE_FACTS_DOMAIN: &[u8] = b"reap.pm.live.account-allowance-facts.v1\0";
const ACCOUNT_SCALAR_KEY_DOMAIN: &[u8] = b"reap.pm.live.account-scalar-key.v1\0";

/// Owner-bound live account completion plus explicitly retained extra-map rows.
pub struct PmLiveAccountSnapshotCompletion {
    pub(crate) delivery: PmCompleteAccountSnapshotDelivery,
    pub(crate) foreign_diagnostics: PmForeignRowDiagnostics,
}

impl PmLiveAccountSnapshotCompletion {
    #[must_use]
    pub const fn foreign_diagnostics(&self) -> PmForeignRowDiagnostics {
        self.foreign_diagnostics
    }

    #[must_use]
    pub fn into_delivery(self) -> PmCompleteAccountSnapshotDelivery {
        self.delivery
    }

    #[must_use]
    pub fn into_parts(self) -> (PmCompleteAccountSnapshotDelivery, PmForeignRowDiagnostics) {
        (self.delivery, self.foreign_diagnostics)
    }
}

pub(crate) struct NormalizedLiveAccountRows {
    pub(crate) balances: [PmFixtureBalanceRow; 2],
    pub(crate) allowances: [PmFixtureAllowanceRow; 2],
    pub(crate) positions: [PmFixturePositionRow; 1],
    pub(crate) foreign_diagnostics: PmForeignRowDiagnostics,
}

pub(crate) fn normalize_live_account(
    scope: LiveNormalizationScope,
    collateral: &PmLiveBalanceAllowance,
    conditional: &PmLiveBalanceAllowance,
) -> Result<NormalizedLiveAccountRows, PmLiveNormalizationError> {
    let domain = scope.instrument.trading_domain();
    let required = domain.required_spenders();
    let collateral_requirement = exact_requirement(required, domain.collateral())?;
    let outcome_requirement = exact_requirement(required, domain.outcome())?;
    let collateral_allowance = collateral
        .exact_allowance(collateral_requirement.spender())
        .ok_or(PmLiveNormalizationError::MissingRequiredAllowance)?;
    let conditional_allowance = conditional
        .exact_allowance(outcome_requirement.spender())
        .ok_or(PmLiveNormalizationError::MissingRequiredAllowance)?;

    let mut diagnostics = ForeignDiagnosticsBuilder::new(ACCOUNT_FOREIGN_DOMAIN, 66);
    retain_extra_allowances(
        &mut diagnostics,
        0,
        collateral_requirement.spender(),
        collateral,
    )?;
    retain_extra_allowances(
        &mut diagnostics,
        1,
        outcome_requirement.spender(),
        conditional,
    )?;
    retain_unscoped_scalar(&mut diagnostics, 0, collateral.unscoped_scalar_present())?;
    retain_unscoped_scalar(&mut diagnostics, 1, conditional.unscoped_scalar_present())?;

    let account = scope.account.handle();
    let collateral_spender = PmSpenderId::new(account, collateral_requirement);
    let outcome_spender = PmSpenderId::new(account, outcome_requirement);
    let lifecycle = scope.instrument.metadata().lifecycle();
    let availability = if lifecycle.active()
        && !lifecycle.closed()
        && !lifecycle.archived()
        && lifecycle.accepting_orders()
        && lifecycle.order_book_enabled()
    {
        PmPositionAvailability::Tradable
    } else {
        PmPositionAvailability::Unavailable
    };

    Ok(NormalizedLiveAccountRows {
        balances: [
            PmFixtureBalanceRow::new(domain.collateral(), collateral.balance()),
            PmFixtureBalanceRow::new(domain.outcome(), conditional.balance()),
        ],
        allowances: [
            PmFixtureAllowanceRow::new(
                collateral_spender,
                PmAllowanceValue::Erc20(collateral_allowance),
            ),
            PmFixtureAllowanceRow::new(
                outcome_spender,
                PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(
                    !conditional_allowance.is_zero(),
                )),
            ),
        ],
        positions: [PmFixturePositionRow::new(
            scope.instrument,
            conditional.balance(),
            availability,
        )],
        foreign_diagnostics: diagnostics.finish()?,
    })
}

fn exact_requirement(
    required: [PmSpenderRequirement; 2],
    asset: PmAssetId,
) -> Result<PmSpenderRequirement, PmLiveNormalizationError> {
    required
        .into_iter()
        .find(|requirement| requirement.asset() == asset)
        .ok_or(PmLiveNormalizationError::MissingRequiredAllowance)
}

fn retain_extra_allowances(
    diagnostics: &mut ForeignDiagnosticsBuilder,
    asset_tag: u8,
    required_spender: reap_pm_core::EvmAddress,
    observed: &PmLiveBalanceAllowance,
) -> Result<(), PmLiveNormalizationError> {
    for entry in observed
        .allowances()
        .iter()
        .copied()
        .filter(|entry| entry.spender() != required_spender)
    {
        let key = semantic_hash(ACCOUNT_ALLOWANCE_KEY_DOMAIN, |digest| {
            digest.update([asset_tag]);
            digest.update(entry.spender().bytes());
        });
        let facts = semantic_hash(ACCOUNT_ALLOWANCE_FACTS_DOMAIN, |digest| {
            digest.update([asset_tag]);
            digest.update(entry.spender().bytes());
            digest.update(entry.amount().to_be_bytes());
        });
        diagnostics.push(key, facts)?;
    }
    Ok(())
}

fn retain_unscoped_scalar(
    diagnostics: &mut ForeignDiagnosticsBuilder,
    asset_tag: u8,
    present: bool,
) -> Result<(), PmLiveNormalizationError> {
    if present {
        let key = semantic_hash(ACCOUNT_SCALAR_KEY_DOMAIN, |digest| {
            digest.update([asset_tag]);
        });
        let facts = semantic_hash(ACCOUNT_SCALAR_KEY_DOMAIN, |digest| {
            digest.update([asset_tag, 1]);
        });
        diagnostics.push(key, facts)?;
    }
    Ok(())
}
