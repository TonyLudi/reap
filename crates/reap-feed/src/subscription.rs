use std::collections::{BTreeMap, HashSet};

use reap_core::{Channel, ConnId, FeedPriority, Subscription, Venue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketPlan {
    pub conn_id: ConnId,
    pub venue: Venue,
    pub private: bool,
    pub subscriptions: Vec<Subscription>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PartitionError {
    #[error("max subscriptions per socket must be positive")]
    ZeroCapacity,
    #[error("subscription connection count must be positive")]
    ZeroConnections,
    #[error("duplicate subscription {channel}/{symbol}")]
    DuplicateSubscription { channel: String, symbol: String },
    #[error("the existing feed subscription partitioner does not support venue {0:?}")]
    UnsupportedVenue(Venue),
}

pub fn partition_subscriptions(
    subscriptions: &[Subscription],
    max_per_socket: usize,
) -> Result<Vec<SocketPlan>, PartitionError> {
    if max_per_socket == 0 {
        return Err(PartitionError::ZeroCapacity);
    }

    let mut groups: BTreeMap<GroupKey, Vec<Subscription>> = BTreeMap::new();
    let mut seen = HashSet::new();
    for subscription in subscriptions {
        if subscription.connections == 0 {
            return Err(PartitionError::ZeroConnections);
        }
        if !seen.insert((
            subscription.venue,
            wire_channel_name(&subscription.channel).to_string(),
            subscription.symbol.clone(),
        )) {
            return Err(PartitionError::DuplicateSubscription {
                channel: wire_channel_name(&subscription.channel).to_string(),
                symbol: subscription
                    .symbol
                    .clone()
                    .unwrap_or_else(|| "<all>".to_string()),
            });
        }
        for replica in 0..subscription.connections {
            let key = GroupKey {
                venue: venue_key(subscription.venue)?,
                channel: channel_key(&subscription.channel),
                priority: priority_key(subscription.priority),
                replica,
                private: subscription.channel.is_private(),
            };
            groups.entry(key).or_default().push(subscription.clone());
        }
    }

    let mut plans = Vec::new();
    for (key, mut group) in groups {
        group.sort_by(|left, right| left.symbol.cmp(&right.symbol));
        let chunk_size = if key.private { 1 } else { max_per_socket };
        for (chunk_index, chunk) in group.chunks(chunk_size).enumerate() {
            let venue = chunk[0].venue;
            let channel = channel_label(&chunk[0].channel);
            let venue_label = venue_label(venue)?;
            plans.push(SocketPlan {
                conn_id: ConnId::new(format!(
                    "{}-{channel}-{}-r{}-{chunk_index}",
                    venue_label,
                    priority_label(chunk[0].priority),
                    key.replica
                )),
                venue,
                private: key.private,
                subscriptions: chunk.to_vec(),
            });
        }
    }
    Ok(plans)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    venue: u8,
    channel: String,
    priority: u8,
    replica: usize,
    private: bool,
}

fn venue_key(venue: Venue) -> Result<u8, PartitionError> {
    match venue {
        Venue::Okx => Ok(1),
        Venue::Polymarket => Err(PartitionError::UnsupportedVenue(Venue::Polymarket)),
    }
}

fn priority_key(priority: FeedPriority) -> u8 {
    match priority {
        FeedPriority::Critical => 1,
        FeedPriority::High => 2,
        FeedPriority::Normal => 3,
        FeedPriority::Low => 4,
    }
}

fn channel_key(channel: &Channel) -> String {
    channel_label(channel)
}

fn venue_label(venue: Venue) -> Result<&'static str, PartitionError> {
    match venue {
        Venue::Okx => Ok("okx"),
        Venue::Polymarket => Err(PartitionError::UnsupportedVenue(Venue::Polymarket)),
    }
}

fn priority_label(priority: FeedPriority) -> &'static str {
    match priority {
        FeedPriority::Critical => "critical",
        FeedPriority::High => "high",
        FeedPriority::Normal => "normal",
        FeedPriority::Low => "low",
    }
}

fn channel_label(channel: &Channel) -> String {
    wire_channel_name(channel).replace(|character: char| !character.is_ascii_alphanumeric(), "_")
}

fn wire_channel_name(channel: &Channel) -> &str {
    match channel {
        Channel::Books => "books",
        Channel::Trades => "trades",
        Channel::Orders => "orders",
        Channel::Fills => "fills",
        Channel::Account => "account",
        Channel::Positions => "positions",
        Channel::Custom(value) => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(symbol: &str) -> Subscription {
        Subscription::public(Venue::Okx, Channel::Books, symbol, FeedPriority::Critical)
    }

    #[test]
    fn old_subscription_partitioning_rejects_polymarket_explicitly() {
        assert_eq!(venue_key(Venue::Okx), Ok(1));
        assert_eq!(venue_label(Venue::Okx), Ok("okx"));
        assert_eq!(
            venue_key(Venue::Polymarket),
            Err(PartitionError::UnsupportedVenue(Venue::Polymarket))
        );
        assert_eq!(
            venue_label(Venue::Polymarket),
            Err(PartitionError::UnsupportedVenue(Venue::Polymarket))
        );

        let subscription = Subscription::public(
            Venue::Polymarket,
            Channel::Books,
            "123",
            FeedPriority::Critical,
        );
        assert_eq!(
            partition_subscriptions(&[subscription], 10),
            Err(PartitionError::UnsupportedVenue(Venue::Polymarket))
        );
    }

    #[test]
    fn partitions_symbols_at_socket_capacity() {
        let subscriptions = [book("BTC-USDT"), book("ETH-USDT"), book("SOL-USDT")];
        let plans = partition_subscriptions(&subscriptions, 2).unwrap();

        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].subscriptions.len(), 2);
        assert_eq!(plans[1].subscriptions.len(), 1);
    }

    #[test]
    fn redundant_subscription_lands_on_distinct_sockets() {
        let mut subscription = book("BTC-USDT");
        subscription.connections = 2;
        let plans = partition_subscriptions(&[subscription], 10).unwrap();

        assert_eq!(plans.len(), 2);
        assert_ne!(plans[0].conn_id, plans[1].conn_id);
    }

    #[test]
    fn private_channels_are_isolated() {
        let orders = Subscription::private(Venue::Okx, Channel::Orders, FeedPriority::Critical);
        let account = Subscription::private(Venue::Okx, Channel::Account, FeedPriority::Critical);
        let plans = partition_subscriptions(&[orders, account], 10).unwrap();

        assert_eq!(plans.len(), 2);
        assert!(plans.iter().all(|plan| plan.private));
    }

    #[test]
    fn duplicate_subscription_is_rejected_before_partitioning() {
        let duplicate = book("BTC-USDT");
        let alias = Subscription::public(
            Venue::Okx,
            Channel::Custom("books".to_string()),
            "BTC-USDT",
            FeedPriority::Low,
        );
        assert_eq!(
            partition_subscriptions(&[duplicate, alias], 10),
            Err(PartitionError::DuplicateSubscription {
                channel: "books".to_string(),
                symbol: "BTC-USDT".to_string(),
            })
        );
    }
}
