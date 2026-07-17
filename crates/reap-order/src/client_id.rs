use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use reap_core::{NewOrder, Side, TimeInForce};
use thiserror::Error;

use crate::authority::RegularApprovalBinding;

const COUNTER_MODULUS: u64 = 2_176_782_336;
const SESSION_MODULUS: u64 = 1_679_616;
const TIMESTAMP_MODULUS: u64 = 101_559_956_668_416;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClientIdError {
    #[error("client order id prefix must contain 1-8 ASCII alphanumeric characters")]
    InvalidPrefix,
}

#[derive(Debug)]
pub struct ClientOrderIdGenerator {
    prefix: String,
    node_id: u16,
    session_id: u64,
    counter: AtomicU64,
    binding: RegularApprovalBinding,
}

/// One-shot proof that a client-order ID came from the bounded generator.
///
/// The value can be inspected for logging and persistence, but only the order
/// authority layer can consume it into a local ownership reservation.
#[derive(Debug, PartialEq, Eq)]
pub struct GeneratedClientOrderId {
    value: String,
    binding: RegularApprovalBinding,
}

impl GeneratedClientOrderId {
    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub(crate) fn binding(&self) -> &RegularApprovalBinding {
        &self.binding
    }

    pub(crate) fn into_string(self) -> String {
        self.value
    }

    #[cfg(test)]
    pub(crate) fn for_test(value: impl Into<String>, binding: RegularApprovalBinding) -> Self {
        Self {
            value: value.into(),
            binding,
        }
    }
}

impl ClientOrderIdGenerator {
    pub(crate) fn new(
        prefix: impl Into<String>,
        node_id: u16,
        binding: RegularApprovalBinding,
    ) -> Result<Self, ClientIdError> {
        let prefix = prefix.into();
        if prefix.is_empty()
            || prefix.len() > 8
            || !prefix
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return Err(ClientIdError::InvalidPrefix);
        }
        Ok(Self {
            prefix,
            node_id,
            session_id: process_session_id(),
            counter: AtomicU64::new(0),
            binding,
        })
    }

    pub fn next(&self, ts_ms: u64) -> GeneratedClientOrderId {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed) % COUNTER_MODULUS;
        let id = format!(
            "{}{}{}{}{}",
            self.prefix,
            padded_base36(self.node_id as u64, 4),
            padded_base36(self.session_id, 4),
            padded_base36(ts_ms % TIMESTAMP_MODULUS, 9),
            padded_base36(counter, 6)
        );
        debug_assert!(id.len() <= 32);
        GeneratedClientOrderId {
            value: id,
            binding: self.binding.clone(),
        }
    }
}

fn process_session_id() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    (nanos ^ u64::from(std::process::id())) % SESSION_MODULUS
}

fn base36(mut value: u64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut reversed = Vec::new();
    while value > 0 {
        let digit = (value % 36) as u8;
        reversed.push(if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        });
        value /= 36;
    }
    reversed.reverse();
    String::from_utf8(reversed).expect("base36 alphabet is valid UTF-8")
}

fn padded_base36(value: u64, width: usize) -> String {
    let value = base36(value);
    format!("{value:0>width$}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reservation {
    New {
        client_order_id: String,
    },
    Pending {
        client_order_id: String,
    },
    Accepted {
        client_order_id: String,
        exchange_order_id: String,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdempotencyError {
    #[error("idempotency key {key:?} was reused with a different order")]
    Conflict { key: String },
    #[error("unknown idempotency key {0:?}")]
    UnknownKey(String),
}

#[derive(Debug, Default)]
pub struct IdempotencyRegistry {
    entries: HashMap<String, IdempotencyEntry>,
}

impl IdempotencyRegistry {
    pub fn reserve(
        &mut self,
        key: impl Into<String>,
        order: &NewOrder,
        new_client_order_id: String,
    ) -> Result<Reservation, IdempotencyError> {
        let key = key.into();
        let fingerprint = OrderFingerprint::from(order);
        if let Some(existing) = self.entries.get(&key) {
            if existing.fingerprint != fingerprint {
                return Err(IdempotencyError::Conflict { key });
            }
            return Ok(match &existing.exchange_order_id {
                Some(exchange_order_id) => Reservation::Accepted {
                    client_order_id: existing.client_order_id.clone(),
                    exchange_order_id: exchange_order_id.clone(),
                },
                None => Reservation::Pending {
                    client_order_id: existing.client_order_id.clone(),
                },
            });
        }
        self.entries.insert(
            key,
            IdempotencyEntry {
                fingerprint,
                client_order_id: new_client_order_id.clone(),
                exchange_order_id: None,
            },
        );
        Ok(Reservation::New {
            client_order_id: new_client_order_id,
        })
    }

    pub fn mark_accepted(
        &mut self,
        key: &str,
        exchange_order_id: impl Into<String>,
    ) -> Result<(), IdempotencyError> {
        let entry = self
            .entries
            .get_mut(key)
            .ok_or_else(|| IdempotencyError::UnknownKey(key.to_string()))?;
        entry.exchange_order_id = Some(exchange_order_id.into());
        Ok(())
    }

    pub fn release_pending(&mut self, key: &str) -> Result<(), IdempotencyError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| IdempotencyError::UnknownKey(key.to_string()))?;
        if entry.exchange_order_id.is_none() {
            self.entries.remove(key);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct IdempotencyEntry {
    fingerprint: OrderFingerprint,
    client_order_id: String,
    exchange_order_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OrderFingerprint {
    symbol: String,
    side: u8,
    qty: u64,
    price: u64,
    time_in_force: u8,
}

impl From<&NewOrder> for OrderFingerprint {
    fn from(order: &NewOrder) -> Self {
        Self {
            symbol: order.symbol.clone(),
            side: match order.side {
                Side::Buy => 1,
                Side::Sell => 2,
            },
            qty: order.qty.to_bits(),
            price: order.price.to_bits(),
            time_in_force: match order.time_in_force {
                TimeInForce::Gtc => 1,
                TimeInForce::Ioc => 2,
                TimeInForce::PostOnly => 3,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(price: f64) -> NewOrder {
        NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        }
    }

    #[test]
    fn generated_ids_are_unique_valid_and_bounded() {
        let generator =
            ClientOrderIdGenerator::new("reap", 7, RegularApprovalBinding::new()).unwrap();
        let first = generator.next(1_700_000_000_000);
        let second = generator.next(1_700_000_000_000);

        assert_ne!(first, second);
        assert!(first.as_str().len() <= 32);
        assert!(
            first
                .as_str()
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        );
    }

    #[test]
    fn idempotency_reuses_pending_and_accepted_ids() {
        let mut registry = IdempotencyRegistry::default();
        assert_eq!(
            registry.reserve("decision-1", &order(100.0), "client-1".to_string()),
            Ok(Reservation::New {
                client_order_id: "client-1".to_string()
            })
        );
        assert_eq!(
            registry.reserve("decision-1", &order(100.0), "unused".to_string()),
            Ok(Reservation::Pending {
                client_order_id: "client-1".to_string()
            })
        );
        registry.mark_accepted("decision-1", "exchange-1").unwrap();
        assert!(matches!(
            registry.reserve("decision-1", &order(100.0), "unused".to_string()),
            Ok(Reservation::Accepted { .. })
        ));
        assert!(matches!(
            registry.reserve("decision-1", &order(101.0), "unused".to_string()),
            Err(IdempotencyError::Conflict { .. })
        ));
    }
}
