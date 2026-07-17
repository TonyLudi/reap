use std::collections::HashMap;

use reap_order::{
    ClientOrderIdGenerator, RegularApprovalScope, RegularExecutionPolicy,
    RegularExecutionPolicyError, RegularExecutionProfile,
};
use reap_strategy::RiskGroupKindConfig;

use crate::{LiveConfig, VerifiedBootstrap};

pub(crate) fn regular_execution_policy(
    config: &LiveConfig,
    verified: &VerifiedBootstrap,
    mut approval_scopes: HashMap<String, RegularApprovalScope>,
) -> Result<
    (
        RegularExecutionPolicy,
        HashMap<String, ClientOrderIdGenerator>,
    ),
    RegularExecutionPolicyError,
> {
    let mut profiles_by_account = HashMap::<String, Vec<RegularExecutionProfile>>::new();
    for configured in &config.strategy.instruments {
        let verified_instrument =
            verified
                .instruments
                .get(&configured.symbol)
                .ok_or_else(|| RegularExecutionPolicyError::MissingInstrument {
                    symbol: configured.symbol.clone(),
                })?;
        let expected_account = config
            .account_for_symbol(&configured.symbol)
            .expect("validated live symbol must have an account owner");
        if verified_instrument.account_id != expected_account.id {
            return Err(RegularExecutionPolicyError::OwnerMismatch {
                symbol: configured.symbol.clone(),
                actual: verified_instrument.account_id.clone(),
                expected: expected_account.id.clone(),
            });
        }
        if expected_account.trade_modes.get(&configured.symbol)
            != Some(&verified_instrument.trade_mode)
        {
            return Err(RegularExecutionPolicyError::TradeModeMismatch {
                symbol: configured.symbol.clone(),
            });
        }
        let reference_only = config
            .strategy
            .risk_groups
            .iter()
            .find(|group| group.name == configured.risk_group)
            .is_some_and(|group| group.kind == RiskGroupKindConfig::RefOnly);
        let quote_allowed =
            !configured.halted && !reference_only && configured.quote_profit_margin < 1.0;
        let hedge_allowed =
            !configured.halted && !reference_only && configured.hedge_profit_margin < 1.0;
        profiles_by_account
            .entry(expected_account.id.clone())
            .or_default()
            .push(RegularExecutionProfile::new(
                configured.symbol.clone(),
                expected_account.id.clone(),
                verified_instrument.risk_model,
                verified_instrument.order_limits,
                verified_instrument.tick_size,
                verified_instrument.lot_size,
                verified_instrument.min_size,
                quote_allowed,
                hedge_allowed,
                verified
                    .quote_stp_verified_accounts
                    .contains(&expected_account.id),
            ));
    }
    let mut unbound_profiles = Vec::new();
    let mut bound_profile_sets = Vec::new();
    let mut client_id_generators = HashMap::new();
    for (account_id, profiles) in profiles_by_account {
        match approval_scopes.get(&account_id) {
            Some(_) => {}
            None => {
                unbound_profiles.extend(profiles);
                continue;
            }
        }
        let scope = approval_scopes
            .remove(&account_id)
            .expect("checked approval scope must exist");
        let account = config
            .accounts
            .iter()
            .find(|account| account.id == account_id)
            .expect("validated execution account must exist");
        let (profile_set, client_id_generator) = scope.bind_profiles_and_client_id_generator(
            profiles,
            &account.id_prefix,
            account.node_id,
        )?;
        bound_profile_sets.push(profile_set);
        client_id_generators.insert(account_id, client_id_generator);
    }
    if let Some((account_id, _)) = approval_scopes.into_iter().next() {
        return Err(RegularExecutionPolicyError::UnknownApprovalAccount { account_id });
    }
    let policy = RegularExecutionPolicy::from_profiles_and_profile_sets(
        unbound_profiles,
        bound_profile_sets,
    )?;
    Ok((policy, client_id_generators))
}
