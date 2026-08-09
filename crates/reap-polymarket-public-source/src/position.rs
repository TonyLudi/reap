use std::{
    fmt,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use reap_pm_core::{EvmAddress, PmConditionId, PmTokenId, U256};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::{PmDataApiPositionScope, PmExactPositionDecimal, PmPublicPositionError};

pub const MAX_POSITION_PAGE_ROWS: usize = 500;
const MAX_ASSET_DECIMAL_BYTES: usize = 78;
const MAX_TITLE_BYTES: usize = 100;
const MAX_SHORT_TEXT_BYTES: usize = 256;
const MAX_ICON_BYTES: usize = 2_048;
const MAX_END_DATE_BYTES: usize = 64;

// Pinned authority: docs/polymarket-controlled-trial-source-manifest.json,
// current_positions, retrieved 2026-08-09, SHA-256
// ff5ae34274c305970f85741997f70149ed284350229a8d417386c3986a62db57.
pub const PM_POSITION_API_SOURCE_AUTHORITY: &str =
    "https://docs.polymarket.com/api-reference/core/get-current-positions-for-a-user.md";
pub const PM_POSITION_API_SOURCE_SHA256: &str =
    "ff5ae34274c305970f85741997f70149ed284350229a8d417386c3986a62db57";

/// Source-owned wall-clock observation captured after receipt of a position
/// page. The completed observation exposes the clock attached to its last
/// page; its commitment binds the receive clock of every page in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmDataApiReceiveClockObservation {
    unix_milliseconds: u64,
}

impl PmDataApiReceiveClockObservation {
    pub(crate) fn capture() -> Result<Self, PmPublicPositionError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PmPublicPositionError::SystemClockBeforeUnixEpoch)?;
        let unix_milliseconds = u64::try_from(elapsed.as_millis())
            .map_err(|_| PmPublicPositionError::SystemClockOutOfRange)?;
        Ok(Self { unix_milliseconds })
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn for_loopback_evidence(unix_milliseconds: u64) -> Self {
        Self { unix_milliseconds }
    }

    #[must_use]
    pub const fn unix_milliseconds(self) -> u64 {
        self.unix_milliseconds
    }
}

/// Secret-free SHA-256 identity of one exact, ordered position observation.
///
/// Only this source can construct the type. It binds the source profile,
/// configured scope, exact ordered page bodies and parsed values, counts,
/// configured-token classification, and source-owned page receive clocks.
/// It is durable correlation evidence, never order-entry authority.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmDataApiPositionObservationCommitment([u8; 32]);

impl PmDataApiPositionObservationCommitment {
    pub(crate) const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for PmDataApiPositionObservationCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("0x")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for PmDataApiPositionObservationCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PmDataApiPositionObservationCommitment")
            .field(&self.to_string())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmDataApiPositionEvidence {
    asset: PmTokenId,
    opposite_asset: PmTokenId,
    size: PmExactPositionDecimal,
    average_price: PmExactPositionDecimal,
    initial_value: PmExactPositionDecimal,
    current_value: PmExactPositionDecimal,
    cash_pnl: PmExactPositionDecimal,
    percent_pnl: PmExactPositionDecimal,
    total_bought: PmExactPositionDecimal,
    realized_pnl: PmExactPositionDecimal,
    percent_realized_pnl: PmExactPositionDecimal,
    current_price: PmExactPositionDecimal,
    outcome_index: u32,
    outcome: Box<str>,
    opposite_outcome: Box<str>,
    redeemable: bool,
    mergeable: bool,
    negative_risk: bool,
}

impl PmDataApiPositionEvidence {
    #[must_use]
    pub const fn asset(&self) -> PmTokenId {
        self.asset
    }

    #[must_use]
    pub const fn opposite_asset(&self) -> PmTokenId {
        self.opposite_asset
    }

    #[must_use]
    pub const fn size(&self) -> &PmExactPositionDecimal {
        &self.size
    }

    /// Exact configured-token position size in the same six-decimal units
    /// used by CLOB orders and conditional-token balance observations.
    pub fn size_protocol_units(&self) -> Result<U256, PmPublicPositionError> {
        self.size.to_protocol_units_exact("size")
    }

    #[must_use]
    pub const fn average_price(&self) -> &PmExactPositionDecimal {
        &self.average_price
    }

    #[must_use]
    pub const fn initial_value(&self) -> &PmExactPositionDecimal {
        &self.initial_value
    }

    #[must_use]
    pub const fn current_value(&self) -> &PmExactPositionDecimal {
        &self.current_value
    }

    #[must_use]
    pub const fn cash_pnl(&self) -> &PmExactPositionDecimal {
        &self.cash_pnl
    }

    #[must_use]
    pub const fn percent_pnl(&self) -> &PmExactPositionDecimal {
        &self.percent_pnl
    }

    #[must_use]
    pub const fn total_bought(&self) -> &PmExactPositionDecimal {
        &self.total_bought
    }

    #[must_use]
    pub const fn realized_pnl(&self) -> &PmExactPositionDecimal {
        &self.realized_pnl
    }

    #[must_use]
    pub const fn percent_realized_pnl(&self) -> &PmExactPositionDecimal {
        &self.percent_realized_pnl
    }

    #[must_use]
    pub const fn current_price(&self) -> &PmExactPositionDecimal {
        &self.current_price
    }

    #[must_use]
    pub const fn outcome_index(&self) -> u32 {
        self.outcome_index
    }

    #[must_use]
    pub fn outcome(&self) -> &str {
        &self.outcome
    }

    #[must_use]
    pub fn opposite_outcome(&self) -> &str {
        &self.opposite_outcome
    }

    #[must_use]
    pub const fn redeemable(&self) -> bool {
        self.redeemable
    }

    #[must_use]
    pub const fn mergeable(&self) -> bool {
        self.mergeable
    }

    #[must_use]
    pub const fn negative_risk(&self) -> bool {
        self.negative_risk
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmConfiguredTokenPosition {
    /// No configured-token row appeared in this monitored page walk. This is
    /// intentionally distinct from a present row whose exact size is zero.
    Absent,
    Present(Box<PmDataApiPositionEvidence>),
}

impl PmConfiguredTokenPosition {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    #[must_use]
    pub fn as_present(&self) -> Option<&PmDataApiPositionEvidence> {
        match self {
            Self::Absent => None,
            Self::Present(position) => Some(position.as_ref()),
        }
    }
}

/// One bounded, monitored Data API projection for the configured token.
///
/// The API has no atomic fence. This value is neither funder-wide inventory
/// completeness nor authority to sell, place, cancel, or sign anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmMonitoredPositionObservation {
    scope: PmDataApiPositionScope,
    pages_observed: u8,
    rows_observed: u16,
    configured_token: PmConfiguredTokenPosition,
    completed_clock: PmDataApiReceiveClockObservation,
    commitment: PmDataApiPositionObservationCommitment,
}

impl PmMonitoredPositionObservation {
    pub(crate) const fn new(
        scope: PmDataApiPositionScope,
        pages_observed: u8,
        rows_observed: u16,
        configured_token: PmConfiguredTokenPosition,
        completed_clock: PmDataApiReceiveClockObservation,
        commitment: PmDataApiPositionObservationCommitment,
    ) -> Self {
        Self {
            scope,
            pages_observed,
            rows_observed,
            configured_token,
            completed_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> PmDataApiPositionScope {
        self.scope
    }

    #[must_use]
    pub const fn pages_observed(&self) -> u8 {
        self.pages_observed
    }

    #[must_use]
    pub const fn rows_observed(&self) -> u16 {
        self.rows_observed
    }

    #[must_use]
    pub const fn configured_token(&self) -> &PmConfiguredTokenPosition {
        &self.configured_token
    }

    #[must_use]
    pub const fn completed_clock(&self) -> PmDataApiReceiveClockObservation {
        self.completed_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmDataApiPositionObservationCommitment {
        self.commitment
    }

    /// This credential-free public observation is never mutation authority.
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPositionRow {
    #[serde(rename = "proxyWallet")]
    proxy_wallet: String,
    asset: String,
    #[serde(rename = "conditionId")]
    condition_id: String,
    size: Box<RawValue>,
    #[serde(rename = "avgPrice")]
    average_price: Box<RawValue>,
    #[serde(rename = "initialValue")]
    initial_value: Box<RawValue>,
    #[serde(rename = "currentValue")]
    current_value: Box<RawValue>,
    #[serde(rename = "cashPnl")]
    cash_pnl: Box<RawValue>,
    #[serde(rename = "percentPnl")]
    percent_pnl: Box<RawValue>,
    #[serde(rename = "totalBought")]
    total_bought: Box<RawValue>,
    #[serde(rename = "realizedPnl")]
    realized_pnl: Box<RawValue>,
    #[serde(rename = "percentRealizedPnl")]
    percent_realized_pnl: Box<RawValue>,
    #[serde(rename = "curPrice")]
    current_price: Box<RawValue>,
    redeemable: bool,
    mergeable: bool,
    title: String,
    slug: String,
    icon: String,
    #[serde(rename = "eventSlug")]
    event_slug: String,
    outcome: String,
    #[serde(rename = "outcomeIndex")]
    outcome_index: Box<RawValue>,
    #[serde(rename = "oppositeOutcome")]
    opposite_outcome: String,
    #[serde(rename = "oppositeAsset")]
    opposite_asset: String,
    #[serde(rename = "endDate")]
    end_date: String,
    #[serde(rename = "negativeRisk")]
    negative_risk: bool,
}

#[derive(Debug)]
pub(crate) struct ParsedPositionRow {
    pub(crate) asset: PmTokenId,
    pub(crate) evidence: PmDataApiPositionEvidence,
}

pub(crate) fn parse_position_page(
    body: &[u8],
    scope: PmDataApiPositionScope,
) -> Result<Vec<ParsedPositionRow>, PmPublicPositionError> {
    let raw_rows: Vec<RawPositionRow> =
        serde_json::from_slice(body).map_err(|_| PmPublicPositionError::InvalidJson)?;
    if raw_rows.len() > MAX_POSITION_PAGE_ROWS {
        return Err(PmPublicPositionError::PageRowLimit);
    }
    raw_rows
        .into_iter()
        .map(|row| parse_position_row(row, scope))
        .collect()
}

fn parse_position_row(
    row: RawPositionRow,
    scope: PmDataApiPositionScope,
) -> Result<ParsedPositionRow, PmPublicPositionError> {
    let proxy_funder = EvmAddress::parse(&row.proxy_wallet)
        .map_err(|_| PmPublicPositionError::InvalidField("proxyWallet"))?;
    if proxy_funder != scope.proxy_funder() {
        return Err(PmPublicPositionError::ScopeMismatch("proxy funder"));
    }
    let condition = PmConditionId::parse(&row.condition_id)
        .map_err(|_| PmPublicPositionError::InvalidField("conditionId"))?;
    if condition != scope.condition() {
        return Err(PmPublicPositionError::ScopeMismatch("condition"));
    }

    validate_text("title", &row.title, MAX_TITLE_BYTES)?;
    validate_text("slug", &row.slug, MAX_SHORT_TEXT_BYTES)?;
    validate_text("icon", &row.icon, MAX_ICON_BYTES)?;
    validate_text("eventSlug", &row.event_slug, MAX_SHORT_TEXT_BYTES)?;
    validate_nonempty_text("outcome", &row.outcome, MAX_SHORT_TEXT_BYTES)?;
    validate_nonempty_text(
        "oppositeOutcome",
        &row.opposite_outcome,
        MAX_SHORT_TEXT_BYTES,
    )?;
    if row.outcome == row.opposite_outcome {
        return Err(PmPublicPositionError::InvalidField("outcome labels"));
    }
    validate_text("endDate", &row.end_date, MAX_END_DATE_BYTES)?;

    let asset = parse_asset("asset", &row.asset)?;
    let opposite_asset = parse_asset("oppositeAsset", &row.opposite_asset)?;
    if asset == opposite_asset {
        return Err(PmPublicPositionError::InvalidField("asset pair"));
    }
    let evidence = PmDataApiPositionEvidence {
        asset,
        opposite_asset,
        size: exact_decimal("size", &row.size, true)?,
        average_price: exact_decimal("avgPrice", &row.average_price, true)?,
        initial_value: exact_decimal("initialValue", &row.initial_value, true)?,
        current_value: exact_decimal("currentValue", &row.current_value, true)?,
        cash_pnl: exact_decimal("cashPnl", &row.cash_pnl, false)?,
        percent_pnl: exact_decimal("percentPnl", &row.percent_pnl, false)?,
        total_bought: exact_decimal("totalBought", &row.total_bought, true)?,
        realized_pnl: exact_decimal("realizedPnl", &row.realized_pnl, false)?,
        percent_realized_pnl: exact_decimal(
            "percentRealizedPnl",
            &row.percent_realized_pnl,
            false,
        )?,
        current_price: exact_decimal("curPrice", &row.current_price, true)?,
        outcome_index: parse_outcome_index(&row.outcome_index)?,
        outcome: row.outcome.into(),
        opposite_outcome: row.opposite_outcome.into(),
        redeemable: row.redeemable,
        mergeable: row.mergeable,
        negative_risk: row.negative_risk,
    };
    Ok(ParsedPositionRow { asset, evidence })
}

fn exact_decimal(
    field: &'static str,
    value: &RawValue,
    nonnegative: bool,
) -> Result<PmExactPositionDecimal, PmPublicPositionError> {
    PmExactPositionDecimal::parse(field, value.get(), nonnegative)
}

fn parse_asset(field: &'static str, value: &str) -> Result<PmTokenId, PmPublicPositionError> {
    if value.len() > MAX_ASSET_DECIMAL_BYTES {
        return Err(PmPublicPositionError::FieldTooLong(field));
    }
    if value.starts_with('0') || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PmPublicPositionError::InvalidField(field));
    }
    let units = U256::from_str(value).map_err(|_| PmPublicPositionError::InvalidField(field))?;
    PmTokenId::new(units).map_err(|_| PmPublicPositionError::InvalidField(field))
}

fn parse_outcome_index(value: &RawValue) -> Result<u32, PmPublicPositionError> {
    let value = value.get();
    if value.is_empty()
        || value.len() > 10
        || (value.starts_with('0') && value.len() != 1)
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(PmPublicPositionError::InvalidField("outcomeIndex"));
    }
    let outcome_index: u32 = value
        .parse()
        .map_err(|_| PmPublicPositionError::InvalidField("outcomeIndex"))?;
    if outcome_index > 1 {
        return Err(PmPublicPositionError::InvalidField("outcomeIndex"));
    }
    Ok(outcome_index)
}

fn validate_nonempty_text(
    field: &'static str,
    value: &str,
    maximum_bytes: usize,
) -> Result<(), PmPublicPositionError> {
    validate_text(field, value, maximum_bytes)?;
    if value.is_empty() {
        return Err(PmPublicPositionError::InvalidField(field));
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    value: &str,
    maximum_bytes: usize,
) -> Result<(), PmPublicPositionError> {
    if value.len() > maximum_bytes {
        return Err(PmPublicPositionError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(PmPublicPositionError::InvalidField(field));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const FUNDER: &str = "0x1111111111111111111111111111111111111111";
    pub(crate) const CONDITION: &str =
        "0x2222222222222222222222222222222222222222222222222222222222222222";

    pub(crate) fn scope(token: u64) -> PmDataApiPositionScope {
        PmDataApiPositionScope::new(
            EvmAddress::parse(FUNDER).unwrap(),
            PmConditionId::parse(CONDITION).unwrap(),
            PmTokenId::new(U256::from_u64(token)).unwrap(),
        )
    }

    pub(crate) fn row(asset: u64, size: &str) -> String {
        let opposite = asset + 1_000_000;
        format!(
            concat!(
                "{{\"proxyWallet\":\"{FUNDER}\",\"asset\":\"{asset}\",",
                "\"conditionId\":\"{CONDITION}\",\"size\":{size},",
                "\"avgPrice\":0.42,\"initialValue\":1.25,",
                "\"currentValue\":1.5,\"cashPnl\":-0.25,",
                "\"percentPnl\":-20,\"totalBought\":3e0,",
                "\"realizedPnl\":0,\"percentRealizedPnl\":0,",
                "\"curPrice\":0.5,\"redeemable\":false,\"mergeable\":false,",
                "\"title\":\"Synthetic market\",\"slug\":\"synthetic\",",
                "\"icon\":\"https://example.invalid/icon\",",
                "\"eventSlug\":\"synthetic-event\",\"outcome\":\"Yes\",",
                "\"outcomeIndex\":0,\"oppositeOutcome\":\"No\",",
                "\"oppositeAsset\":\"{opposite}\",",
                "\"endDate\":\"2030-01-01T00:00:00Z\",\"negativeRisk\":false}}"
            ),
            FUNDER = FUNDER,
            CONDITION = CONDITION,
            asset = asset,
            size = size,
            opposite = opposite,
        )
    }

    #[test]
    fn exact_row_parser_retains_all_numeric_lexemes() {
        let raw = format!("[{}]", row(7, "0"));
        let rows = parse_position_page(raw.as_bytes(), scope(7)).unwrap();
        let evidence = &rows[0].evidence;
        assert_eq!(evidence.asset().units(), U256::from_u64(7));
        assert_eq!(evidence.size().lexeme(), "0");
        assert!(evidence.size().is_zero());
        assert_eq!(evidence.size_protocol_units(), Ok(U256::ZERO));
        assert_eq!(evidence.average_price().lexeme(), "0.42");
        assert_eq!(evidence.cash_pnl().lexeme(), "-0.25");
        assert_eq!(evidence.total_bought().lexeme(), "3e0");
        assert_eq!(evidence.outcome_index(), 0);
        assert_eq!(evidence.outcome(), "Yes");
        assert_eq!(evidence.opposite_outcome(), "No");

        let fractional = format!("[{}]", row(7, "12.3400e-2"));
        let rows = parse_position_page(fractional.as_bytes(), scope(7)).unwrap();
        assert_eq!(
            rows[0].evidence.size_protocol_units(),
            Ok(U256::from_u64(123_400))
        );
    }

    #[test]
    fn parser_rejects_scope_conflict_type_confusion_and_unknown_fields() {
        let wrong_funder =
            row(7, "1").replace(FUNDER, "0x3333333333333333333333333333333333333333");
        assert_eq!(
            parse_position_page(format!("[{wrong_funder}]").as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::ScopeMismatch("proxy funder")
        );

        let quoted_size = row(7, "\"1\"");
        assert_eq!(
            parse_position_page(format!("[{quoted_size}]").as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::InvalidField("size")
        );

        let unknown = row(7, "1").replacen('{', "{\"unexpected\":1,", 1);
        assert_eq!(
            parse_position_page(format!("[{unknown}]").as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::InvalidJson
        );
    }

    #[test]
    fn parser_rejects_oversized_pages_assets_and_numeric_evidence() {
        let page = format!(
            "[{}]",
            (1..=MAX_POSITION_PAGE_ROWS + 1)
                .map(|asset| row(asset as u64, "1"))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(
            parse_position_page(page.as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::PageRowLimit
        );

        let oversized_number = "1".repeat(crate::MAX_POSITION_DECIMAL_BYTES + 1);
        assert_eq!(
            parse_position_page(
                format!("[{}]", row(7, &oversized_number)).as_bytes(),
                scope(7),
            )
            .unwrap_err(),
            PmPublicPositionError::FieldTooLong("size")
        );

        let invalid_asset = row(7, "1").replace("\"asset\":\"7\"", "\"asset\":\"007\"");
        assert_eq!(
            parse_position_page(format!("[{invalid_asset}]").as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::InvalidField("asset")
        );
    }

    #[test]
    fn parser_rejects_non_binary_or_ambiguous_two_token_identity() {
        let equal_asset =
            row(7, "1").replace("\"oppositeAsset\":\"1000007\"", "\"oppositeAsset\":\"7\"");
        assert_eq!(
            parse_position_page(format!("[{equal_asset}]").as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::InvalidField("asset pair")
        );

        let invalid_index = row(7, "1").replace("\"outcomeIndex\":0", "\"outcomeIndex\":2");
        assert_eq!(
            parse_position_page(format!("[{invalid_index}]").as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::InvalidField("outcomeIndex")
        );

        let empty_label = row(7, "1").replace("\"outcome\":\"Yes\"", "\"outcome\":\"\"");
        assert_eq!(
            parse_position_page(format!("[{empty_label}]").as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::InvalidField("outcome")
        );

        let equal_labels =
            row(7, "1").replace("\"oppositeOutcome\":\"No\"", "\"oppositeOutcome\":\"Yes\"");
        assert_eq!(
            parse_position_page(format!("[{equal_labels}]").as_bytes(), scope(7)).unwrap_err(),
            PmPublicPositionError::InvalidField("outcome labels")
        );
    }

    #[test]
    fn source_authority_constant_is_pinned() {
        assert_eq!(
            PM_POSITION_API_SOURCE_AUTHORITY,
            "https://docs.polymarket.com/api-reference/core/get-current-positions-for-a-user.md"
        );
        assert_eq!(
            PM_POSITION_API_SOURCE_SHA256,
            "ff5ae34274c305970f85741997f70149ed284350229a8d417386c3986a62db57"
        );
    }
}
