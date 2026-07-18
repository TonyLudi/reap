use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use reap_core::{Channel, ConnId, FeedPriority, Subscription, Venue};
use reap_feed::SocketPlan;
use reap_order::okx_order_dispatch_key;

use crate::{
    ChaosConnectivityPlan, LiveConfig, LiveMode, PrivateChannelPlan, PublicChannelPlan,
    PublicRedundancyConsumer, RequirementUse,
};

use super::LiveRuntimeError;
use super::dispatch::order_dispatch_lane;

#[derive(Debug, Clone)]
pub(super) struct PlannedPublicSubscription {
    pub(super) subscription: Subscription,
    pub(super) redundancy_consumer: Option<PublicRedundancyConsumer>,
    pub(super) requirements: Vec<RequirementUse>,
}

pub(super) fn validate_runtime_connectivity_plan(
    config: &LiveConfig,
    plan: &ChaosConnectivityPlan,
    mode: LiveMode,
) -> Result<(), LiveRuntimeError> {
    if plan.mode() != mode {
        return Err(LiveRuntimeError::Subscription(format!(
            "connectivity plan mode {:?} does not match runtime mode {mode:?}",
            plan.mode()
        )));
    }
    if plan.environment() != config.venue.environment {
        return Err(LiveRuntimeError::Subscription(format!(
            "connectivity plan environment {:?} does not match config environment {:?}",
            plan.environment(),
            config.venue.environment
        )));
    }
    let config_accounts = config
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if plan.account_ids() != config_accounts {
        return Err(LiveRuntimeError::Subscription(
            "connectivity plan account boundary does not match the live config".to_string(),
        ));
    }
    let config_symbols = config
        .strategy
        .instruments
        .iter()
        .map(|instrument| instrument.symbol.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if plan.symbols() != config_symbols {
        return Err(LiveRuntimeError::Subscription(
            "connectivity plan symbol boundary does not match the live config".to_string(),
        ));
    }
    let maximum_public_replicas = plan
        .public_subscriptions()
        .iter()
        .map(|subscription| usize::from(subscription.replica_count()))
        .max()
        .unwrap_or_default();
    if maximum_public_replicas > config.runtime.public_connection_replica_cap() {
        return Err(LiveRuntimeError::Subscription(format!(
            "connectivity plan requires {maximum_public_replicas} public replicas, exceeding the configured safety ceiling {}",
            config.runtime.public_connection_replica_cap()
        )));
    }
    let mut command_shards_by_account = HashMap::<&str, usize>::new();
    for lane in plan.command_lanes() {
        *command_shards_by_account
            .entry(lane.account_id())
            .or_default() += 1;
    }
    let maximum_order_shards = command_shards_by_account
        .into_values()
        .max()
        .unwrap_or_default();
    if maximum_order_shards > config.runtime.order_command_shard_cap() {
        return Err(LiveRuntimeError::Subscription(format!(
            "connectivity plan requires {maximum_order_shards} order shards, exceeding the configured safety ceiling {}",
            config.runtime.order_command_shard_cap()
        )));
    }
    Ok(())
}

pub(super) fn runtime_public_subscriptions(
    plan: &ChaosConnectivityPlan,
) -> Result<Vec<PlannedPublicSubscription>, LiveRuntimeError> {
    let mut subscriptions = Vec::with_capacity(plan.public_subscriptions().len());
    let mut seen = HashSet::new();
    for planned in plan.public_subscriptions() {
        if planned.replica_count() == 0 {
            return Err(LiveRuntimeError::Subscription(format!(
                "planned public subscription {:?}/{} has zero replicas",
                planned.channel(),
                planned.symbol()
            )));
        }
        if planned.requirements().is_empty() {
            return Err(LiveRuntimeError::Subscription(format!(
                "planned public subscription {:?}/{} has no Chaos requirement",
                planned.channel(),
                planned.symbol()
            )));
        }
        if planned.session_surfaces().is_empty()
            || planned.channel_surface().capability_id() != planned.channel().capability_id()
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "planned public subscription {:?}/{} has invalid capability metadata",
                planned.channel(),
                planned.symbol()
            )));
        }
        if (planned.replica_count() > 1) != planned.redundancy_consumer().is_some() {
            return Err(LiveRuntimeError::Subscription(format!(
                "planned public subscription {:?}/{} has replicas without exact redundancy-consumer metadata",
                planned.channel(),
                planned.symbol()
            )));
        }
        let channel = match planned.channel() {
            PublicChannelPlan::Books => Channel::Books,
            PublicChannelPlan::Trades => Channel::Trades,
            PublicChannelPlan::FundingRate
            | PublicChannelPlan::IndexTickers
            | PublicChannelPlan::MarkPrice
            | PublicChannelPlan::PriceLimit => {
                Channel::Custom(planned.channel_surface().endpoint_or_channel().to_string())
            }
        };
        if !seen.insert((channel.clone(), planned.symbol().to_string())) {
            return Err(LiveRuntimeError::Subscription(format!(
                "connectivity plan repeats public subscription {:?}/{}",
                planned.channel(),
                planned.symbol()
            )));
        }
        let priority = if planned.channel() == PublicChannelPlan::Trades {
            FeedPriority::High
        } else {
            FeedPriority::Critical
        };
        let mut subscription =
            Subscription::public(Venue::Okx, channel, planned.symbol(), priority);
        subscription.connections = usize::from(planned.replica_count());
        subscriptions.push(PlannedPublicSubscription {
            subscription,
            redundancy_consumer: planned.redundancy_consumer(),
            requirements: planned.requirements().to_vec(),
        });
    }
    if subscriptions.is_empty() {
        return Err(LiveRuntimeError::Subscription(
            "connectivity plan has no public subscriptions".to_string(),
        ));
    }
    Ok(subscriptions)
}

pub(super) fn validate_public_socket_plans(
    subscriptions: &[PlannedPublicSubscription],
    socket_plans: &[SocketPlan],
) -> Result<(), LiveRuntimeError> {
    let mut occurrences = HashMap::<(Channel, Option<String>), usize>::new();
    for socket in socket_plans {
        if socket.private || socket.venue != Venue::Okx {
            return Err(LiveRuntimeError::Subscription(
                "public connectivity plan produced a private or non-OKX socket".to_string(),
            ));
        }
        for subscription in &socket.subscriptions {
            *occurrences
                .entry((subscription.channel.clone(), subscription.symbol.clone()))
                .or_default() += 1;
        }
    }
    if occurrences.len() != subscriptions.len() {
        return Err(LiveRuntimeError::Subscription(
            "public socket plan contains an unplanned subscription".to_string(),
        ));
    }
    for planned in subscriptions {
        let key = (
            planned.subscription.channel.clone(),
            planned.subscription.symbol.clone(),
        );
        let actual = occurrences.get(&key).copied().unwrap_or_default();
        if actual != planned.subscription.connections {
            return Err(LiveRuntimeError::Subscription(format!(
                "public socket plan materialized {actual} replicas for {:?}/{}, expected {}",
                planned.subscription.channel,
                planned.subscription.symbol.as_deref().unwrap_or("<all>"),
                planned.subscription.connections
            )));
        }
        let consumers = planned
            .requirements
            .iter()
            .map(RequirementUse::consumer)
            .collect::<BTreeSet<_>>();
        if consumers.is_empty() || (actual > 1) != planned.redundancy_consumer.is_some() {
            return Err(LiveRuntimeError::Subscription(format!(
                "public socket plan lost consumer metadata for {:?}/{}",
                planned.subscription.channel,
                planned.subscription.symbol.as_deref().unwrap_or("<all>")
            )));
        }
    }
    Ok(())
}

pub(super) fn private_socket_plans_by_account(
    plan: &ChaosConnectivityPlan,
) -> Result<BTreeMap<String, Vec<SocketPlan>>, LiveRuntimeError> {
    let mut plans_by_account = BTreeMap::new();
    for session in plan.private_state_sessions() {
        validate_private_state_socket_count(session.account_id(), session.socket_count())?;
        if session.channels().is_empty()
            || session.requirements().is_empty()
            || session.session_surfaces().is_empty()
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "private state session for {} is empty or has incomplete metadata",
                session.account_id()
            )));
        }
        if !plan
            .account_ids()
            .iter()
            .any(|account_id| account_id == session.account_id())
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "private state session references unknown account {}",
                session.account_id()
            )));
        }
        let mut channels = Vec::with_capacity(session.channels().len());
        let mut seen_channels = HashSet::new();
        let mut binding_requirements = BTreeSet::new();
        for binding in session.channels() {
            if binding.requirements().is_empty()
                || binding.surface().capability_id() != binding.channel().capability_id()
            {
                return Err(LiveRuntimeError::Subscription(format!(
                    "private state channel {:?} for {} has incomplete requirement metadata",
                    binding.channel(),
                    session.account_id()
                )));
            }
            let channel = match binding.channel() {
                PrivateChannelPlan::Account => Channel::Account,
                PrivateChannelPlan::Fills => Channel::Fills,
                PrivateChannelPlan::Orders => Channel::Orders,
                PrivateChannelPlan::Positions => Channel::Positions,
            };
            if !seen_channels.insert(channel.clone()) {
                return Err(LiveRuntimeError::Subscription(format!(
                    "private state session for {} repeats channel {:?}",
                    session.account_id(),
                    channel
                )));
            }
            binding_requirements.extend(binding.requirements().iter().cloned());
            channels.push(Subscription::private(
                Venue::Okx,
                channel,
                FeedPriority::Critical,
            ));
        }
        let session_requirements = session
            .requirements()
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if binding_requirements != session_requirements {
            return Err(LiveRuntimeError::Subscription(format!(
                "private state session for {} does not preserve its channel consumers",
                session.account_id()
            )));
        }
        let account_plans = vec![SocketPlan {
            conn_id: ConnId::new(format!("okx-private-{}-r0", session.account_id())),
            venue: Venue::Okx,
            private: true,
            subscriptions: channels,
        }];
        if plans_by_account
            .insert(session.account_id().to_string(), account_plans)
            .is_some()
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "connectivity plan repeats private state session for {}",
                session.account_id()
            )));
        }
    }
    let planned_accounts = plans_by_account.keys().cloned().collect::<Vec<_>>();
    if planned_accounts != plan.account_ids() {
        return Err(LiveRuntimeError::Subscription(
            "connectivity plan must define exactly one private state session per account"
                .to_string(),
        ));
    }
    Ok(plans_by_account)
}

pub(super) fn validate_private_state_socket_count(
    account_id: &str,
    socket_count: u16,
) -> Result<(), LiveRuntimeError> {
    if socket_count != 1 {
        return Err(LiveRuntimeError::Subscription(format!(
            "private state session for {account_id} must use exactly one socket, configured {socket_count}"
        )));
    }
    Ok(())
}

pub(super) fn planned_order_session_counts(
    plan: &ChaosConnectivityPlan,
) -> Result<BTreeMap<String, usize>, LiveRuntimeError> {
    if plan.mode() != LiveMode::Demo && !plan.command_lanes().is_empty() {
        return Err(LiveRuntimeError::Subscription(
            "order command lanes are only valid in demo mode".to_string(),
        ));
    }
    let planned_accounts = plan
        .account_ids()
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut lanes_by_account = BTreeMap::<String, Vec<_>>::new();
    for lane in plan.command_lanes() {
        if !planned_accounts.contains(lane.account_id()) {
            return Err(LiveRuntimeError::Subscription(format!(
                "order command lane references unknown account {}",
                lane.account_id()
            )));
        }
        if lane.dispatch_families().is_empty()
            || lane.requirements().is_empty()
            || lane.session_surfaces().is_empty()
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "order command lane {} for {} is empty or has incomplete consumer metadata",
                lane.lane_index(),
                lane.account_id()
            )));
        }
        lanes_by_account
            .entry(lane.account_id().to_string())
            .or_default()
            .push(lane);
    }
    let mut counts = BTreeMap::new();
    for (account_id, mut lanes) in lanes_by_account {
        lanes.sort_by_key(|lane| lane.lane_index());
        let lane_count = lanes.len();
        if lane_count != 1 {
            return Err(LiveRuntimeError::Subscription(format!(
                "account {account_id} must have exactly one order command lane, found {lane_count}"
            )));
        }
        let mut families = BTreeSet::new();
        for (expected_index, lane) in lanes.into_iter().enumerate() {
            if usize::from(lane.lane_index()) != expected_index {
                return Err(LiveRuntimeError::Subscription(format!(
                    "account {account_id} order lane indices must be contiguous from zero"
                )));
            }
            for family in lane.dispatch_families() {
                if family.trim().is_empty()
                    || okx_order_dispatch_key(family) != *family
                    || !families.insert(family.clone())
                {
                    return Err(LiveRuntimeError::Subscription(format!(
                        "account {account_id} has an invalid or duplicate order dispatch family {family:?}"
                    )));
                }
                if order_dispatch_lane(family, lane_count) != expected_index {
                    return Err(LiveRuntimeError::Subscription(format!(
                        "account {account_id} dispatch family {family} does not route to planned lane {expected_index}"
                    )));
                }
            }
        }
        counts.insert(account_id, lane_count);
    }
    Ok(counts)
}
