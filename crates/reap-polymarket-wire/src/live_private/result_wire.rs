use std::{collections::BTreeMap, fmt};

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawPlaceResult {
    pub(super) success: bool,
    #[serde(rename = "orderID")]
    pub(super) order_id: String,
    pub(super) status: String,
    #[serde(rename = "errorMsg")]
    pub(super) error_message: String,
    #[serde(rename = "makingAmount")]
    pub(super) making_amount: String,
    #[serde(rename = "takingAmount")]
    pub(super) taking_amount: String,
    #[serde(rename = "tradeIDs")]
    pub(super) trade_ids: Vec<String>,
    #[serde(default, rename = "transactionsHashes")]
    pub(super) transaction_hashes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawCancelResult {
    pub(super) canceled: Vec<String>,
    pub(super) not_canceled: RawUniqueStringMap,
}

pub(super) struct RawUniqueStringMap(pub(super) BTreeMap<String, String>);

struct RawUniqueStringMapVisitor;

impl<'de> Visitor<'de> for RawUniqueStringMapVisitor {
    type Value = RawUniqueStringMap;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a map with unique string keys and string values")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some((key, value)) = map.next_entry::<String, String>()? {
            if values.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate not_canceled identity"));
            }
        }
        Ok(RawUniqueStringMap(values))
    }
}

impl<'de> Deserialize<'de> for RawUniqueStringMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(RawUniqueStringMapVisitor)
    }
}
