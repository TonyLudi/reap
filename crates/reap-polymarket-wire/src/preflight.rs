//! Strict wire parsing for the two PM-T2 read-only safety preflights.
//!
//! The response contracts are frozen from the 2026-08-09 v3 official-source
//! capture: `api-reference/geoblock.md` and the closed-only section of
//! `trading/manage-orders.md`. Neither parser owns transport or mutation
//! authority.

use std::net::IpAddr;

use serde::Deserialize;

use crate::PmWireError;

pub const MAX_PM_GEOBLOCK_BODY_BYTES: usize = 512;
pub const MAX_PM_CLOSED_ONLY_BODY_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmGeoblockStatus {
    blocked: bool,
    ip: IpAddr,
    country: Box<str>,
    region: Box<str>,
}

impl PmGeoblockStatus {
    #[must_use]
    pub const fn blocked(&self) -> bool {
        self.blocked
    }

    #[must_use]
    pub const fn ip(&self) -> IpAddr {
        self.ip
    }

    #[must_use]
    pub fn country(&self) -> &str {
        &self.country
    }

    #[must_use]
    pub fn region(&self) -> &str {
        &self.region
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmClosedOnlyStatus {
    closed_only: bool,
}

impl PmClosedOnlyStatus {
    #[must_use]
    pub const fn closed_only(self) -> bool {
        self.closed_only
    }
}

pub fn parse_pm_geoblock(raw: &[u8]) -> Result<PmGeoblockStatus, PmWireError> {
    if raw.len() > MAX_PM_GEOBLOCK_BODY_BYTES {
        return Err(PmWireError::RestBodyTooLarge);
    }
    let wire =
        serde_json::from_slice::<RawGeoblock>(raw).map_err(|_| PmWireError::MalformedJson)?;
    let blocked = wire.blocked.ok_or(PmWireError::MissingField("blocked"))?;
    let ip_text = wire.ip.ok_or(PmWireError::MissingField("ip"))?;
    let ip = ip_text
        .parse::<IpAddr>()
        .map_err(|_| PmWireError::InvalidIdentity("ip"))?;
    if ip.to_string() != ip_text {
        return Err(PmWireError::InvalidIdentity("ip"));
    }
    let country = wire.country.ok_or(PmWireError::MissingField("country"))?;
    if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(PmWireError::InvalidIdentity("country"));
    }
    let region = wire.region.ok_or(PmWireError::MissingField("region"))?;
    if region.len() > 16
        || !region
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(PmWireError::InvalidIdentity("region"));
    }
    Ok(PmGeoblockStatus {
        blocked,
        ip,
        country: country.into_boxed_str(),
        region: region.into_boxed_str(),
    })
}

pub fn parse_pm_closed_only(raw: &[u8]) -> Result<PmClosedOnlyStatus, PmWireError> {
    if raw.len() > MAX_PM_CLOSED_ONLY_BODY_BYTES {
        return Err(PmWireError::RestBodyTooLarge);
    }
    let wire =
        serde_json::from_slice::<RawClosedOnly>(raw).map_err(|_| PmWireError::MalformedJson)?;
    Ok(PmClosedOnlyStatus {
        closed_only: wire
            .closed_only
            .ok_or(PmWireError::MissingField("closed_only"))?,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGeoblock {
    #[serde(default)]
    blocked: Option<bool>,
    #[serde(default)]
    ip: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    region: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClosedOnly {
    #[serde(default)]
    closed_only: Option<bool>,
}
