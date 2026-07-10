use std::collections::BTreeMap;

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
}

pub fn partition_subscriptions(
    subscriptions: &[Subscription],
    max_per_socket: usize,
) -> Result<Vec<SocketPlan>, PartitionError> {
    if max_per_socket == 0 {
        return Err(PartitionError::ZeroCapacity);
    }

    let mut groups: BTreeMap<GroupKey, Vec<Subscription>> = BTreeMap::new();
    for subscription in subscriptions {
        if subscription.connections == 0 {
            return Err(PartitionError::ZeroConnections);
        }
        for replica in 0..subscription.connections {
            let key = GroupKey {
                venue: venue_key(subscription.venue),
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
            plans.push(SocketPlan {
                conn_id: ConnId::new(format!(
                    "{}-{channel}-{}-r{}-{chunk_index}",
                    venue_label(venue),
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

fn venue_key(venue: Venue) -> u8 {
    match venue {
        Venue::Okx => 1,
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

fn venue_label(venue: Venue) -> &'static str {
    match venue {
        Venue::Okx => "okx",
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
    match channel {
        Channel::Books => "books".to_string(),
        Channel::Trades => "trades".to_string(),
        Channel::Orders => "orders".to_string(),
        Channel::Fills => "fills".to_string(),
        Channel::Account => "account".to_string(),
        Channel::Custom(value) => {
            value.replace(|character: char| !character.is_ascii_alphanumeric(), "_")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(symbol: &str) -> Subscription {
        Subscription::public(Venue::Okx, Channel::Books, symbol, FeedPriority::Critical)
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
}
