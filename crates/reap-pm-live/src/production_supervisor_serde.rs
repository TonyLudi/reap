//! Stable private serialization helpers shared by supervisor journal facts.

use reap_pm_core::PmOrderSide;
use serde::{Deserialize as _, Deserializer, Serializer};

pub(crate) fn serialize<S>(side: &PmOrderSide, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(match side {
        PmOrderSide::Buy => "buy",
        PmOrderSide::Sell => "sell",
    })
}

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<PmOrderSide, D::Error>
where
    D: Deserializer<'de>,
{
    match <&str>::deserialize(deserializer)? {
        "buy" => Ok(PmOrderSide::Buy),
        "sell" => Ok(PmOrderSide::Sell),
        _ => Err(serde::de::Error::custom("invalid Polymarket order side")),
    }
}
