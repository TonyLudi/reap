use std::collections::{BTreeMap, BTreeSet};

use reap_core::{MarketEvent, NormalizedEvent, SystemEventKind};
use reap_strategy::{ReferenceDataKind, RiskGroupKindConfig};

use super::{InitializationArtifactV1, SeedRoute, positive_finite};

pub(super) fn validate_seeded_clean_profile(
    artifact: &InitializationArtifactV1,
) -> Result<(), String> {
    validate_clean_transition_state(artifact)?;
    validate_metadata_agreement(artifact)?;
    validate_seed_routes_and_clocks(artifact)?;
    validate_account_seeds(artifact)?;
    validate_declared_seed_projection(artifact)
}

fn validate_clean_transition_state(artifact: &InitializationArtifactV1) -> Result<(), String> {
    let state = &artifact.declared_state;
    let zero_f64 = |value: f64| value.to_bits() == 0.0_f64.to_bits();
    if state.kill_switch_reason.is_some()
        || !state.halted_symbols.is_empty()
        || !state.live_orders.is_empty()
        || !state.order_rejections.is_empty()
        || !state.rejected_order_ids.is_empty()
        || state.last_order_rejection_ms != 0
        || !state.unfilled_ioc_cancellations.is_empty()
        || !state.unfilled_ioc_cancelled_order_ids.is_empty()
        || state.last_unfilled_ioc_cancel_ms != 0
        || !zero_f64(state.turnover_usd)
        || !state.seen_fills.is_empty()
        || !state.stablecoin_breach_since.is_empty()
    {
        return Err(
            "schema-1 initialization requires an explicit clean transition-history genesis"
                .to_string(),
        );
    }
    for account in &artifact.accounts {
        if !account.baseline_fill_ids.is_empty() {
            return Err(format!(
                "schema-1 account {} must declare an empty baseline fill set",
                account.id
            ));
        }
    }
    Ok(())
}

fn validate_metadata_agreement(artifact: &InitializationArtifactV1) -> Result<(), String> {
    for initialized in &artifact.instruments {
        let configured = artifact
            .strategy
            .instruments
            .iter()
            .find(|instrument| instrument.symbol == initialized.symbol)
            .expect("base validation requires exact symbol coverage");
        let configured_group = artifact
            .strategy
            .risk_groups
            .iter()
            .find(|group| group.name == configured.risk_group)
            .ok_or_else(|| {
                format!(
                    "instrument {} references missing strategy risk group {}",
                    configured.symbol, configured.risk_group
                )
            })?;
        if configured_group.account_id.as_deref() != Some(initialized.account_id.as_str()) {
            return Err(format!(
                "instrument {} live account differs from strategy risk-group account",
                initialized.symbol
            ));
        }
        if configured.tick_size.to_bits() != initialized.tick_size.to_bits()
            || configured.lot_size.to_bits() != initialized.lot_size.to_bits()
            || configured.min_trade_size.to_bits() != initialized.min_size.to_bits()
        {
            return Err(format!(
                "instrument {} live increments differ from effective strategy metadata",
                initialized.symbol
            ));
        }
        let expected_type = if configured.kind.is_spot() {
            "spot"
        } else if configured.kind.is_swap() {
            "swap"
        } else {
            "futures"
        };
        if initialized.instrument_type != expected_type {
            return Err(format!(
                "instrument {} type {} differs from effective strategy kind",
                initialized.symbol, initialized.instrument_type
            ));
        }
        if configured.kind.is_spot() {
            if initialized.contract_value.is_some()
                || !matches!(initialized.risk_model, reap_risk::InstrumentRiskModel::Spot)
            {
                return Err(format!(
                    "spot instrument {} has derivative risk metadata",
                    initialized.symbol
                ));
            }
        } else {
            let expected_contract = configured.contract_value;
            if initialized
                .contract_value
                .is_none_or(|value| value.to_bits() != expected_contract.to_bits())
            {
                return Err(format!(
                    "derivative instrument {} contract value differs from strategy",
                    initialized.symbol
                ));
            }
            let model_contract = match initialized.risk_model {
                reap_risk::InstrumentRiskModel::LinearDerivative { contract_value }
                    if !configured.kind.is_inverse() =>
                {
                    contract_value
                }
                reap_risk::InstrumentRiskModel::InverseDerivative { contract_value }
                    if configured.kind.is_inverse() =>
                {
                    contract_value
                }
                _ => {
                    return Err(format!(
                        "instrument {} risk model differs from effective strategy kind",
                        initialized.symbol
                    ));
                }
            };
            if model_contract.to_bits() != expected_contract.to_bits() {
                return Err(format!(
                    "instrument {} risk-model contract differs from strategy",
                    initialized.symbol
                ));
            }
        }
    }

    for account in &artifact.accounts {
        if account.id_prefix.is_empty()
            || account.id_prefix.len() > 8
            || !account
                .id_prefix
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return Err(format!(
                "account {} client-order prefix must contain 1-8 ASCII alphanumeric characters",
                account.id
            ));
        }
        let has_quote_capable_profile = artifact.strategy.instruments.iter().any(|instrument| {
            artifact.instruments.iter().any(|initialized| {
                initialized.account_id == account.id && initialized.symbol == instrument.symbol
            }) && !instrument.halted
                && instrument.quote_profit_margin < 1.0
                && artifact
                    .strategy
                    .risk_groups
                    .iter()
                    .find(|group| group.name == instrument.risk_group)
                    .is_none_or(|group| group.kind != RiskGroupKindConfig::RefOnly)
        });
        if has_quote_capable_profile && !account.quote_stp_verified {
            return Err(format!(
                "account {} lacks required quote STP verification",
                account.id
            ));
        }
        if !matches!(
            account.expected_account_level.as_str(),
            "spot"
                | "futures"
                | "multi_currency_margin"
                | "portfolio_margin"
                | "single_currency_margin"
        ) {
            return Err(format!(
                "account {} has unsupported account level {}",
                account.id, account.expected_account_level
            ));
        }
        if !matches!(
            account.expected_position_mode.as_str(),
            "long_short_mode" | "net_mode"
        ) {
            return Err(format!(
                "account {} has unsupported position mode {}",
                account.id, account.expected_position_mode
            ));
        }
        for (symbol, mode) in &account.trade_modes {
            if !matches!(mode.as_str(), "cash" | "cross" | "isolated") {
                return Err(format!(
                    "account {} symbol {} has unsupported trade mode {}",
                    account.id, symbol, mode
                ));
            }
            let initialized = artifact
                .instruments
                .iter()
                .find(|instrument| instrument.symbol == *symbol)
                .expect("base validation matches trade modes to instruments");
            if initialized.trade_mode != *mode {
                return Err(format!(
                    "account {} symbol {} trade mode differs from instrument metadata",
                    account.id, symbol
                ));
            }
        }
        validate_account_update_numbers(&account.id, &account.bootstrap_update)?;
    }
    Ok(())
}

fn validate_seed_routes_and_clocks(artifact: &InitializationArtifactV1) -> Result<(), String> {
    let mut previous_arrival_ns = 0;
    let mut previous_observed_ms = 0;
    for seed in &artifact.seed_events {
        if seed.arrival_ns < previous_arrival_ns || seed.observed_now_ms < previous_observed_ms {
            return Err(format!(
                "seed {} moves the local initialization clock backwards",
                seed.sequence
            ));
        }
        previous_arrival_ns = seed.arrival_ns;
        previous_observed_ms = seed.observed_now_ms;
        if seed.event.ts_ms() > seed.observed_now_ms {
            return Err(format!(
                "seed {} source time exceeds its observed local time",
                seed.sequence
            ));
        }
        match (&seed.route, &seed.event) {
            (SeedRoute::PrivateAccount, NormalizedEvent::Account(update)) => {
                validate_account_update_numbers("seed", update)?;
            }
            (SeedRoute::PrivateAccount, _) => {
                return Err(format!(
                    "private-account seed {} is not an Account event",
                    seed.sequence
                ));
            }
            (SeedRoute::Normalized, NormalizedEvent::Account(_)) => {
                return Err(format!(
                    "Account seed {} must use the private-account route",
                    seed.sequence
                ));
            }
            (SeedRoute::Normalized, NormalizedEvent::Market(market)) => {
                validate_market_seed(seed.sequence, market)?;
            }
            (SeedRoute::Normalized, NormalizedEvent::System(system))
                if matches!(
                    system.kind,
                    SystemEventKind::FeedRecovered
                        | SystemEventKind::PrivateStreamRecovered
                        | SystemEventKind::OrderTransportRecovered
                ) => {}
            (SeedRoute::Normalized, NormalizedEvent::System(system)) => {
                return Err(format!(
                    "schema-1 clean genesis forbids seed system transition {:?}",
                    system.kind
                ));
            }
            (
                SeedRoute::Normalized,
                NormalizedEvent::Order(_) | NormalizedEvent::Timer(_) | NormalizedEvent::Control(_),
            ) => {
                return Err(format!(
                    "schema-1 clean genesis forbids seed event {}",
                    seed.sequence
                ));
            }
        }
    }
    let last = artifact
        .seed_events
        .last()
        .expect("base validation requires seed events");
    if artifact.declared_state.source_clock.seed_arrival_ns != last.arrival_ns
        || artifact.declared_state.source_clock.seed_now_ms != last.observed_now_ms
    {
        return Err("declared source clock must equal the final seed clock".to_string());
    }
    Ok(())
}

fn validate_market_seed(sequence: u64, market: &MarketEvent) -> Result<(), String> {
    match market {
        MarketEvent::Depth(book) => {
            if book.symbol.trim().is_empty() || book.bids.is_empty() || book.asks.is_empty() {
                return Err(format!(
                    "seed {sequence} depth requires a symbol and both book sides"
                ));
            }
            for (name, levels) in [("bid", &book.bids), ("ask", &book.asks)] {
                if levels
                    .iter()
                    .any(|level| !positive_finite(level.px) || !positive_finite(level.qty))
                {
                    return Err(format!(
                        "seed {sequence} depth has a non-positive or non-finite {name} level"
                    ));
                }
            }
            if book
                .bids
                .windows(2)
                .any(|levels| levels[0].px <= levels[1].px)
                || book
                    .asks
                    .windows(2)
                    .any(|levels| levels[0].px >= levels[1].px)
                || book.best_bid().expect("checked bid").px
                    >= book.best_ask().expect("checked ask").px
            {
                return Err(format!(
                    "seed {sequence} depth is duplicated, unsorted, or crossed"
                ));
            }
        }
        MarketEvent::IndexPrice { symbol, price, .. } => {
            if symbol.trim().is_empty() || !positive_finite(*price) {
                return Err(format!("seed {sequence} has an invalid index price"));
            }
        }
        MarketEvent::FundingRate {
            symbol,
            rate,
            funding_time_ms,
            settlement,
            ..
        } => {
            if symbol.trim().is_empty() || !rate.is_finite() || *funding_time_ms == 0 {
                return Err(format!("seed {sequence} has an invalid funding rate"));
            }
            if settlement.as_ref().is_some_and(|settlement| {
                settlement.funding_time_ms == 0 || !settlement.rate.is_finite()
            }) {
                return Err(format!("seed {sequence} has an invalid funding settlement"));
            }
        }
        MarketEvent::PriceLimits {
            symbol,
            mark_price,
            limit_down,
            limit_up,
            ..
        } => {
            if symbol.trim().is_empty()
                || !positive_finite(*mark_price)
                || !positive_finite(*limit_down)
                || !positive_finite(*limit_up)
                || limit_down > mark_price
                || mark_price > limit_up
            {
                return Err(format!("seed {sequence} has invalid price limits"));
            }
        }
        MarketEvent::Trade { .. } | MarketEvent::BurstSignal { .. } => {
            return Err(format!(
                "schema-1 clean genesis forbids stateful market seed {sequence}"
            ));
        }
    }
    Ok(())
}

fn validate_account_seeds(artifact: &InitializationArtifactV1) -> Result<(), String> {
    let initialized_accounts = artifact
        .accounts
        .iter()
        .map(|account| account.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut authoritative = BTreeMap::new();
    for seed in &artifact.seed_events {
        let (SeedRoute::PrivateAccount, NormalizedEvent::Account(update)) =
            (&seed.route, &seed.event)
        else {
            continue;
        };
        let account_id = scoped_account_update_id(update).map_err(|error| {
            format!(
                "private-account seed {} has invalid account scope: {error}",
                seed.sequence
            )
        })?;
        if !initialized_accounts.contains(account_id) {
            return Err(format!(
                "private-account seed {} references unknown account {account_id}",
                seed.sequence
            ));
        }
        validate_account_update_rows(&format!("private-account seed {}", seed.sequence), update)?;
        if authoritative.insert(account_id, update).is_some() {
            return Err(format!(
                "account {account_id} has more than one authoritative private seed"
            ));
        }
    }

    for account in &artifact.accounts {
        validate_account_update_rows(
            &format!("account {} bootstrap", account.id),
            &account.bootstrap_update,
        )?;
        if scoped_account_update_id(&account.bootstrap_update)? != account.id {
            return Err(format!(
                "account {} bootstrap snapshot has a different account scope",
                account.id
            ));
        }
        let matching = authoritative.get(account.id.as_str()).ok_or_else(|| {
            format!(
                "account {} requires exactly one authoritative private seed",
                account.id
            )
        })?;
        if serde_json::to_value(matching).map_err(|error| error.to_string())?
            != serde_json::to_value(&account.bootstrap_update).map_err(|error| error.to_string())?
        {
            return Err(format!(
                "account {} bootstrap snapshot differs from its private seed",
                account.id
            ));
        }
    }
    Ok(())
}

fn validate_declared_seed_projection(artifact: &InitializationArtifactV1) -> Result<(), String> {
    validate_feed_projection(artifact)?;
    validate_private_projection(artifact)?;
    validate_order_transport_projection(artifact)?;
    validate_mark_projection(artifact)?;
    validate_position_and_equity_projection(artifact)?;
    validate_stablecoin_projection(artifact)?;
    validate_strategy_reference_projection(artifact)
}

fn validate_order_transport_projection(artifact: &InitializationArtifactV1) -> Result<(), String> {
    let mut recovered = BTreeSet::new();
    for seed in &artifact.seed_events {
        let NormalizedEvent::System(system) = &seed.event else {
            continue;
        };
        if system.kind != SystemEventKind::OrderTransportRecovered {
            continue;
        }
        let account_id = system.account_id.as_deref().ok_or_else(|| {
            format!(
                "order-transport recovery seed {} has no account",
                seed.sequence
            )
        })?;
        if !recovered.insert(account_id) {
            return Err(format!(
                "account {account_id} has duplicate order-transport recovery seeds"
            ));
        }
    }
    let declared = artifact
        .live
        .order_transport_ready_accounts
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if recovered != declared {
        return Err("declared order-transport readiness differs from seed transitions".to_string());
    }
    Ok(())
}

fn validate_feed_projection(artifact: &InitializationArtifactV1) -> Result<(), String> {
    let mut actual = BTreeMap::<(String, String), (u64, bool)>::new();
    for seed in &artifact.seed_events {
        let NormalizedEvent::System(system) = &seed.event else {
            continue;
        };
        let (Some(venue), Some(symbol)) = (system.venue, system.symbol.as_ref()) else {
            continue;
        };
        let key = (venue_key(venue)?, symbol.clone());
        match system.kind {
            SystemEventKind::FeedRecovered | SystemEventKind::FeedHeartbeat => {
                actual.insert(key, (system.ts_ms, false));
            }
            SystemEventKind::FeedStale
            | SystemEventKind::FeedGap
            | SystemEventKind::BookRecoveryStarted
            | SystemEventKind::BookRecoveryFailed => {
                actual.entry(key).or_insert((0, true)).1 = true;
            }
            _ => {}
        }
    }
    let declared = ordered_map(
        "feed_health",
        artifact.declared_state.feed_health.iter().map(|row| {
            Ok((
                (venue_key(row.venue)?, row.symbol.clone()),
                (row.last_ready_ms, row.stale),
            ))
        }),
    )?;
    if actual != declared {
        return Err("declared feed health differs from seed transitions".to_string());
    }
    let now_ms = artifact.declared_state.source_clock.seed_now_ms;
    if actual.values().any(|(last_ready_ms, stale)| {
        *stale || now_ms.saturating_sub(*last_ready_ms) > artifact.risk_limits.max_feed_age_ms
    }) {
        return Err("schema-1 clean genesis feed health is stale".to_string());
    }
    Ok(())
}

fn validate_private_projection(artifact: &InitializationArtifactV1) -> Result<(), String> {
    let mut actual = BTreeMap::<(String, Option<String>), (u64, bool)>::new();
    for seed in &artifact.seed_events {
        let NormalizedEvent::System(system) = &seed.event else {
            continue;
        };
        let Some(venue) = system.venue else {
            continue;
        };
        let key = (venue_key(venue)?, system.account_id.clone());
        match system.kind {
            SystemEventKind::PrivateStreamRecovered | SystemEventKind::PrivateStreamHeartbeat => {
                actual.insert(key, (system.ts_ms, false));
            }
            SystemEventKind::PrivateStreamStale | SystemEventKind::ReconcileDrift => {
                actual.entry(key).or_insert((0, true)).1 = true;
            }
            _ => {}
        }
    }
    let declared = ordered_map(
        "private_health",
        artifact.declared_state.private_health.iter().map(|row| {
            Ok((
                (venue_key(row.venue)?, row.account_id.clone()),
                (row.last_ready_ms, row.stale),
            ))
        }),
    )?;
    if actual != declared {
        return Err("declared private health differs from seed transitions".to_string());
    }
    let now_ms = artifact.declared_state.source_clock.seed_now_ms;
    if actual.values().any(|(last_ready_ms, stale)| {
        *stale || now_ms.saturating_sub(*last_ready_ms) > artifact.risk_limits.max_private_age_ms
    }) {
        return Err("schema-1 clean genesis private health is stale".to_string());
    }
    Ok(())
}

fn validate_mark_projection(artifact: &InitializationArtifactV1) -> Result<(), String> {
    let mut actual = BTreeMap::new();
    for seed in &artifact.seed_events {
        if let NormalizedEvent::Market(MarketEvent::Depth(book)) = &seed.event {
            let mark = book
                .mid()
                .filter(|value| positive_finite(*value))
                .ok_or_else(|| format!("seed {} has invalid risk mark", seed.sequence))?;
            actual.insert(book.symbol.clone(), mark.to_bits());
        }
    }
    let declared = ordered_map(
        "marks",
        artifact
            .declared_state
            .marks
            .iter()
            .map(|row| Ok((row.symbol.clone(), row.value.to_bits()))),
    )?;
    if actual != declared {
        return Err("declared risk marks differ from seeded depth mids".to_string());
    }
    Ok(())
}

fn validate_position_and_equity_projection(
    artifact: &InitializationArtifactV1,
) -> Result<(), String> {
    let mut positions = BTreeMap::new();
    let mut equity_by_account = BTreeMap::<Option<String>, u64>::new();
    let mut peak_equity = 0.0_f64;
    for seed in &artifact.seed_events {
        let NormalizedEvent::Account(update) = &seed.event else {
            continue;
        };
        for position in &update.positions {
            positions.insert(position.symbol.clone(), position.qty.to_bits());
        }
        for margin in &update.margins {
            if let Some(equity) = margin.adjusted_equity_usd {
                equity_by_account.insert(margin.account_id.clone(), equity.to_bits());
            }
        }
        let aggregate = equity_by_account
            .values()
            .map(|bits| f64::from_bits(*bits))
            .sum::<f64>();
        peak_equity = peak_equity.max(aggregate);
    }
    let declared_positions = ordered_map(
        "positions",
        artifact
            .declared_state
            .positions
            .iter()
            .map(|row| Ok((row.symbol.clone(), row.value.to_bits()))),
    )?;
    if positions != declared_positions {
        return Err("declared positions differ from account seeds".to_string());
    }
    let declared_equity = ordered_map(
        "equity_by_account",
        artifact
            .declared_state
            .equity_by_account
            .iter()
            .map(|row| Ok((row.account_id.clone(), row.equity_usd.to_bits()))),
    )?;
    let aggregate = equity_by_account
        .values()
        .map(|bits| f64::from_bits(*bits))
        .sum::<f64>();
    if equity_by_account != declared_equity
        || aggregate.to_bits() != artifact.declared_state.equity_usd.to_bits()
        || peak_equity.to_bits() != artifact.declared_state.peak_equity_usd.to_bits()
    {
        return Err("declared equity differs from account seeds".to_string());
    }
    Ok(())
}

fn validate_stablecoin_projection(artifact: &InitializationArtifactV1) -> Result<(), String> {
    let guards = artifact
        .risk_limits
        .stablecoin_guards
        .iter()
        .map(|guard| guard.symbol.as_str())
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeMap::<String, (u64, u64, bool)>::new();
    for seed in &artifact.seed_events {
        let NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms,
            symbol,
            price,
        }) = &seed.event
        else {
            continue;
        };
        if !guards.contains(symbol.as_str()) {
            continue;
        }
        match actual.get_mut(symbol) {
            Some((current_ts, current_price, conflict)) if *ts_ms < *current_ts => {}
            Some((current_ts, current_price, conflict)) if *ts_ms == *current_ts => {
                if price.to_bits() != *current_price {
                    *conflict = true;
                }
            }
            Some(state) => {
                *state = (*ts_ms, price.to_bits(), false);
            }
            None => {
                actual.insert(symbol.clone(), (*ts_ms, price.to_bits(), false));
            }
        }
    }
    let declared = ordered_map(
        "stablecoin_rates",
        artifact.declared_state.stablecoin_rates.iter().map(|row| {
            Ok((
                row.symbol.clone(),
                (row.ts_ms, row.price.to_bits(), row.conflict),
            ))
        }),
    )?;
    let missing = guards
        .difference(&actual.keys().map(String::as_str).collect::<BTreeSet<_>>())
        .copied()
        .collect::<BTreeSet<_>>();
    let declared_missing = artifact
        .declared_state
        .stablecoin_missing_symbols
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for guard in &artifact.risk_limits.stablecoin_guards {
        let Some((rate_ts_ms, price_bits, conflict)) = actual.get(&guard.symbol) else {
            continue;
        };
        let price = f64::from_bits(*price_bits);
        let minimum = 1.0 - guard.max_downside_deviation;
        let age_ms = artifact
            .declared_state
            .source_clock
            .seed_now_ms
            .saturating_sub(*rate_ts_ms);
        if *conflict
            || !positive_finite(price)
            || price < minimum
            || age_ms > artifact.risk_limits.stablecoin_max_age_ms
        {
            return Err(format!(
                "schema-1 clean genesis stablecoin {} is conflicted, stale, or below {}",
                guard.symbol, minimum,
            ));
        }
    }
    if actual != declared || missing != declared_missing {
        return Err("declared stablecoin state differs from market seeds".to_string());
    }
    Ok(())
}

fn validate_strategy_reference_projection(
    artifact: &InitializationArtifactV1,
) -> Result<(), String> {
    let requirement_rows = artifact.strategy.reference_data_requirements();
    let requirement_ages = requirement_rows
        .iter()
        .map(|requirement| {
            (
                (
                    requirement.kind.as_str().to_string(),
                    requirement.symbol.clone(),
                ),
                requirement.max_age_ms,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let requirements = requirement_rows
        .into_iter()
        .map(|requirement| {
            (
                (requirement.kind.as_str().to_string(), requirement.symbol),
                None,
            )
        })
        .collect::<BTreeMap<_, Option<(u64, u64)>>>();
    let mut actual = requirements;
    for seed in &artifact.seed_events {
        let NormalizedEvent::Market(market) = &seed.event else {
            continue;
        };
        let mut observe = |kind: ReferenceDataKind, symbol: &str, source_ts_ms| {
            if let Some(state) = actual.get_mut(&(kind.as_str().to_string(), symbol.to_string()))
                && state.is_none_or(|(current_ts_ms, _)| source_ts_ms >= current_ts_ms)
            {
                *state = Some((source_ts_ms, seed.observed_now_ms));
            }
        };
        match market {
            MarketEvent::IndexPrice {
                ts_ms,
                symbol,
                price,
            } if positive_finite(*price) => {
                observe(ReferenceDataKind::IndexPrice, symbol, *ts_ms);
            }
            MarketEvent::FundingRate {
                ts_ms,
                symbol,
                rate,
                funding_time_ms,
                ..
            } if rate.is_finite() && *funding_time_ms > 0 => {
                observe(ReferenceDataKind::FundingRate, symbol, *ts_ms);
            }
            MarketEvent::PriceLimits {
                ts_ms,
                symbol,
                mark_price,
                limit_down,
                limit_up,
            } => {
                if positive_finite(*mark_price) {
                    observe(ReferenceDataKind::MarkPrice, symbol, *ts_ms);
                }
                if positive_finite(*limit_down) && positive_finite(*limit_up) {
                    observe(ReferenceDataKind::PriceLimits, symbol, *ts_ms);
                }
            }
            _ => {}
        }
    }
    if actual.values().any(Option::is_none) {
        return Err("seed events do not satisfy every strategy reference".to_string());
    }
    let now_ms = artifact.declared_state.source_clock.seed_now_ms;
    for (key, state) in &actual {
        let (source_ts_ms, _) = state.expect("checked complete strategy reference");
        let max_age_ms = requirement_ages
            .get(key)
            .expect("same requirements created both maps");
        if now_ms.saturating_sub(source_ts_ms) > *max_age_ms {
            return Err(format!(
                "schema-1 clean genesis strategy reference {}:{} is stale",
                key.0, key.1
            ));
        }
    }
    let declared = ordered_map(
        "strategy_references",
        artifact
            .declared_state
            .strategy_references
            .iter()
            .map(|row| {
                Ok((
                    (row.kind.clone(), row.symbol.clone()),
                    Some((row.source_ts_ms, row.observed_now_ms)),
                ))
            }),
    )?;
    if actual != declared {
        return Err("declared strategy references differ from market seeds".to_string());
    }
    Ok(())
}

fn validate_account_update_numbers(
    context: &str,
    update: &reap_core::AccountUpdate,
) -> Result<(), String> {
    validate_account_update_rows(context, update)?;
    for balance in &update.balances {
        for (name, value) in [
            ("total", balance.total),
            ("available", balance.available),
            ("equity", balance.equity),
            ("liability", balance.liability),
            ("max_loan", balance.max_loan),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{context} balance {name} is invalid"));
            }
        }
        if balance
            .forced_repayment_indicator
            .is_some_and(|indicator| indicator != 0)
        {
            return Err(format!(
                "{context} cannot seed an active forced-repayment indicator"
            ));
        }
    }
    for position in &update.positions {
        if !position.qty.is_finite() || !position.avg_price.is_finite() {
            return Err(format!(
                "{context} position {} is non-finite",
                position.symbol
            ));
        }
    }
    for margin in &update.margins {
        for value in [
            margin.ratio,
            margin.exchange_ratio,
            margin.adjusted_equity_usd,
            margin.notional_usd,
        ]
        .into_iter()
        .flatten()
        {
            if !value.is_finite() {
                return Err(format!("{context} margin is non-finite"));
            }
        }
    }
    Ok(())
}

fn validate_account_update_rows(
    context: &str,
    update: &reap_core::AccountUpdate,
) -> Result<(), String> {
    let mut balance_currencies = BTreeSet::new();
    for balance in &update.balances {
        if balance.currency.trim().is_empty()
            || !balance_currencies.insert(balance.currency.as_str())
        {
            return Err(format!(
                "{context} has a duplicate or empty balance currency"
            ));
        }
    }
    let mut position_symbols = BTreeSet::new();
    for position in &update.positions {
        if position.symbol.trim().is_empty() || !position_symbols.insert(position.symbol.as_str()) {
            return Err(format!(
                "{context} has a duplicate or empty position symbol"
            ));
        }
    }
    let mut margin_accounts = BTreeSet::new();
    for margin in &update.margins {
        if !margin_accounts.insert(margin.account_id.as_deref()) {
            return Err(format!("{context} has duplicate margin rows"));
        }
    }
    Ok(())
}

fn scoped_account_update_id(update: &reap_core::AccountUpdate) -> Result<&str, String> {
    let mut ids = BTreeSet::new();
    for account_id in update
        .balances
        .iter()
        .map(|balance| balance.account_id.as_deref())
        .chain(
            update
                .margins
                .iter()
                .map(|margin| margin.account_id.as_deref()),
        )
    {
        let account_id = account_id
            .ok_or_else(|| "every balance and margin must name its account".to_string())?;
        ids.insert(account_id);
    }
    if ids.len() != 1 {
        return Err(
            "an account update must contain scoped rows for exactly one account".to_string(),
        );
    }
    Ok(ids
        .into_iter()
        .next()
        .expect("a single account scope was checked"))
}

fn venue_key(venue: reap_core::Venue) -> Result<String, String> {
    serde_json::to_value(venue)
        .map_err(|error| format!("serialize venue: {error}"))?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "venue serialization is not a string".to_string())
}

fn ordered_map<K, V>(
    name: &str,
    rows: impl Iterator<Item = Result<(K, V), String>>,
) -> Result<BTreeMap<K, V>, String>
where
    K: Clone + Ord + std::fmt::Debug,
{
    let mut map = BTreeMap::new();
    let mut previous = None;
    for row in rows {
        let (key, value) = row?;
        if previous.as_ref().is_some_and(|previous| previous >= &key) {
            return Err(format!(
                "{name} rows are duplicated or not in canonical order at {key:?}"
            ));
        }
        previous = Some(key.clone());
        map.insert(key, value);
    }
    Ok(map)
}
