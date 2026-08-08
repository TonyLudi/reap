use std::fmt;

use serde::{
    Deserialize, Deserializer,
    de::{SeqAccess, Visitor},
};

pub(super) struct RawAssociateTrades(pub(super) Vec<String>);

struct RawAssociateTradesVisitor;

impl<'de> Visitor<'de> for RawAssociateTradesVisitor {
    type Value = RawAssociateTrades;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("null or a list of associated trade identities")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(RawAssociateTrades(Vec::new()))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<String>()? {
            values.push(value);
        }
        Ok(RawAssociateTrades(values))
    }
}

impl<'de> Deserialize<'de> for RawAssociateTrades {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawAssociateTradesVisitor)
    }
}
