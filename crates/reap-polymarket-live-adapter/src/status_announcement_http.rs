use std::{
    collections::{HashMap, HashSet},
    fmt,
    time::Duration,
};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;

use crate::{
    PM_STATUS_PRODUCTION_ORIGIN, PmHttpReceiveClock, PmLiveAdapterError,
    config::{OriginMode, PmStatusHttpConfig},
    http_transport::{PmHttpTransport, PmPublicRoute},
    observation_clock::PmHttpReceiveClockSource,
};

/// Strict response-body bound for the current-status summary source.
pub const MAX_PM_STATUS_SUMMARY_BODY_BYTES: usize = 256 * 1024;
/// Strict response-body bound for the current component source.
pub const MAX_PM_STATUS_COMPONENTS_BODY_BYTES: usize = 512 * 1024;
pub const MAX_PM_STATUS_ACTIVE_INCIDENTS: usize = 64;
pub const MAX_PM_STATUS_ACTIVE_MAINTENANCES: usize = 64;
pub const MAX_PM_STATUS_COMPONENTS: usize = 256;

const MAX_STATUS_ID_BYTES: usize = 128;
const MAX_STATUS_NAME_BYTES: usize = 512;
const MAX_STATUS_DESCRIPTION_BYTES: usize = 4 * 1024;
const MAX_STATUS_URL_BYTES: usize = 2 * 1024;
const MAX_STATUS_TIMESTAMP_BYTES: usize = 30;
const STATUS_ANNOUNCEMENT_SCHEMA: &[u8] =
    b"summary-object-v3+components-wrapper-object-v3/current-announcements-only";
const STATUS_ANNOUNCEMENT_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.status-announcement-observation.v1\0";

/// Closed error surface for the ordered current-announcement observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmStatusAnnouncementError {
    #[error(transparent)]
    Http(#[from] PmLiveAdapterError),
    #[error("status announcement response is invalid: {0}")]
    InvalidResponse(&'static str),
}

/// Overall current status reported by the status page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmStatusPageState {
    Up,
    HasIssues,
    UnderMaintenance,
}

/// Current component state reported by the status page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmStatusComponentState {
    Operational,
    UnderMaintenance,
    DegradedPerformance,
    PartialOutage,
    MajorOutage,
}

/// Lifecycle state retained for one summary active-incident row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmStatusIncidentState {
    Investigating,
    Identified,
    Monitoring,
    Resolved,
}

/// Impact retained for one summary active-incident row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmStatusIncidentImpact {
    None,
    MinorOutage,
    DegradedPerformance,
    PartialOutage,
    MajorOutage,
}

/// Lifecycle state retained for one summary active-maintenance row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmStatusMaintenanceState {
    NotStartedYet,
    InProgress,
    Completed,
}

/// Exact typed page row from the current summary response.
pub struct PmStatusPageAnnouncement {
    name: Box<str>,
    url: Box<str>,
    state: PmStatusPageState,
}

impl PmStatusPageAnnouncement {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub const fn state(&self) -> PmStatusPageState {
        self.state
    }
}

/// Exact typed active-incident row from the current summary response.
pub struct PmStatusActiveIncident {
    id: Box<str>,
    name: Box<str>,
    started_utc: Box<str>,
    state: PmStatusIncidentState,
    impact: PmStatusIncidentImpact,
    url: Box<str>,
    updated_at_utc: Box<str>,
}

impl PmStatusActiveIncident {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn started_utc(&self) -> &str {
        &self.started_utc
    }

    #[must_use]
    pub const fn state(&self) -> PmStatusIncidentState {
        self.state
    }

    #[must_use]
    pub const fn impact(&self) -> PmStatusIncidentImpact {
        self.impact
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub fn updated_at_utc(&self) -> &str {
        &self.updated_at_utc
    }
}

/// Exact typed active-maintenance row from the current summary response.
pub struct PmStatusActiveMaintenance {
    id: Box<str>,
    name: Box<str>,
    start_utc: Box<str>,
    state: PmStatusMaintenanceState,
    duration_minutes: u32,
    url: Box<str>,
    updated_at_utc: Box<str>,
}

impl PmStatusActiveMaintenance {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn start_utc(&self) -> &str {
        &self.start_utc
    }

    #[must_use]
    pub const fn state(&self) -> PmStatusMaintenanceState {
        self.state
    }

    #[must_use]
    pub const fn duration_minutes(&self) -> u32 {
        self.duration_minutes
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub fn updated_at_utc(&self) -> &str {
        &self.updated_at_utc
    }
}

/// Exact typed group row nested in a current component response.
pub struct PmStatusComponentGroup {
    id: Box<str>,
    name: Box<str>,
    description: Box<str>,
}

impl PmStatusComponentGroup {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Exact typed component row from the current components response.
///
/// The live `/v3/components.json` response is an object containing a
/// `components` array whose reviewed rows are exactly
/// `id/name/description/status/group`. The status-page public-API example
/// currently shows a stale bare-array shape with `isParent/children`; this
/// parser intentionally rejects that alternate shape and any unreviewed issue
/// fields. Active incident and maintenance details come only from the summary.
pub struct PmStatusComponentAnnouncement {
    id: Box<str>,
    name: Box<str>,
    description: Box<str>,
    state: PmStatusComponentState,
    group: Option<PmStatusComponentGroup>,
}

impl PmStatusComponentAnnouncement {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub const fn state(&self) -> PmStatusComponentState {
        self.state
    }

    #[must_use]
    pub const fn group(&self) -> Option<&PmStatusComponentGroup> {
        self.group.as_ref()
    }
}

/// One domain-separated commitment to the ordered summary/components cut,
/// including both exact raw bounded responses and both receive edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmStatusAnnouncementObservationCommitment([u8; 32]);

impl PmStatusAnnouncementObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Move-only ordered observation of the fixed current-announcement sources.
///
/// This retains the complete typed current summary and component rows. It is
/// announcement evidence only: the documented summary excludes components
/// and historical notices, and neither endpoint proves matching-engine state,
/// restart absence, restricted/cancel-only/post-only absence, order admission,
/// account state, or a shared egress with another connection.
pub struct PmStatusAnnouncementObservation {
    page: PmStatusPageAnnouncement,
    active_incidents: Vec<PmStatusActiveIncident>,
    active_maintenances: Vec<PmStatusActiveMaintenance>,
    components: Vec<PmStatusComponentAnnouncement>,
    summary_receive_clock: PmHttpReceiveClock,
    components_receive_clock: PmHttpReceiveClock,
    commitment: PmStatusAnnouncementObservationCommitment,
}

impl PmStatusAnnouncementObservation {
    #[must_use]
    pub const fn page(&self) -> &PmStatusPageAnnouncement {
        &self.page
    }

    #[must_use]
    pub fn active_incidents(&self) -> &[PmStatusActiveIncident] {
        &self.active_incidents
    }

    #[must_use]
    pub fn active_maintenances(&self) -> &[PmStatusActiveMaintenance] {
        &self.active_maintenances
    }

    #[must_use]
    pub fn components(&self) -> &[PmStatusComponentAnnouncement] {
        &self.components
    }

    #[must_use]
    pub const fn summary_receive_clock(&self) -> PmHttpReceiveClock {
        self.summary_receive_clock
    }

    #[must_use]
    pub const fn components_receive_clock(&self) -> PmHttpReceiveClock {
        self.components_receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmStatusAnnouncementObservationCommitment {
        self.commitment
    }
}

impl fmt::Debug for PmStatusAnnouncementObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmStatusAnnouncementObservation(<ordered-summary-components; announcement-only; sealed>)",
        )
    }
}

/// Move-only proof that an ordered announcement observation came from the
/// role's private production-origin mode. LocalEvidence cannot construct it.
pub struct PmProductionStatusAnnouncementObservation {
    observation: PmStatusAnnouncementObservation,
}

impl PmProductionStatusAnnouncementObservation {
    fn from_source(
        _production_origin: ProductionStatusOrigin,
        observation: PmStatusAnnouncementObservation,
    ) -> Self {
        Self { observation }
    }

    #[must_use]
    pub const fn page(&self) -> &PmStatusPageAnnouncement {
        self.observation.page()
    }

    #[must_use]
    pub fn active_incidents(&self) -> &[PmStatusActiveIncident] {
        self.observation.active_incidents()
    }

    #[must_use]
    pub fn active_maintenances(&self) -> &[PmStatusActiveMaintenance] {
        self.observation.active_maintenances()
    }

    #[must_use]
    pub fn components(&self) -> &[PmStatusComponentAnnouncement] {
        self.observation.components()
    }

    #[must_use]
    pub const fn summary_receive_clock(&self) -> PmHttpReceiveClock {
        self.observation.summary_receive_clock()
    }

    #[must_use]
    pub const fn components_receive_clock(&self) -> PmHttpReceiveClock {
        self.observation.components_receive_clock()
    }

    #[must_use]
    pub const fn commitment(&self) -> PmStatusAnnouncementObservationCommitment {
        self.observation.commitment()
    }
}

impl fmt::Debug for PmProductionStatusAnnouncementObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmProductionStatusAnnouncementObservation(<production-origin; announcement-only; sealed>)",
        )
    }
}

struct ProductionStatusOrigin;

impl ProductionStatusOrigin {
    fn verify(mode: OriginMode) -> Result<Self, PmLiveAdapterError> {
        match mode {
            OriginMode::Production => Ok(Self),
            #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
            OriginMode::LocalEvidence => Err(PmLiveAdapterError::InvalidConfiguration(
                "production status announcement observation requires the fixed production origin",
            )),
        }
    }
}

/// Purpose-closed current Polymarket status-announcement role.
///
/// Production construction accepts only timeouts. It has no caller-selected
/// origin, path, method, body, retry, proxy, hash, or clock. The independent
/// status-page connection does not prove the egress used by the CLOB health
/// role or by any authenticated connection.
pub struct PmStatusAnnouncementHttpRole {
    transport: PmHttpTransport,
    mode: OriginMode,
    clock: PmHttpReceiveClockSource,
}

impl PmStatusAnnouncementHttpRole {
    pub fn production(
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::from_config(PmStatusHttpConfig::production(
            connect_timeout,
            request_timeout,
        )?)
    }

    #[cfg(any(test, feature = "read-only-evidence"))]
    pub fn read_only_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::from_config(PmStatusHttpConfig::local_evidence(
            origin,
            connect_timeout,
            request_timeout,
        )?)
    }

    fn from_config(config: PmStatusHttpConfig) -> Result<Self, PmLiveAdapterError> {
        let mode = config.mode();
        Ok(Self {
            transport: PmHttpTransport::status(&config)?,
            mode,
            clock: PmHttpReceiveClockSource::system(),
        })
    }

    /// Fetch the exact current status cut in fixed order: summary first, then
    /// components. Each receive edge is sampled after its bounded body is
    /// complete. There is no retry or caller-selected request surface.
    pub async fn ordered_announcement_observation(
        &self,
    ) -> Result<PmStatusAnnouncementObservation, PmStatusAnnouncementError> {
        let summary_body = self
            .transport
            .get(
                PmPublicRoute::StatusSummary,
                MAX_PM_STATUS_SUMMARY_BODY_BYTES,
            )
            .await?;
        let summary_receive_clock = self.clock.observe()?;
        let summary = parse_status_summary(&summary_body)?;

        let components_body = self
            .transport
            .get(
                PmPublicRoute::StatusComponents,
                MAX_PM_STATUS_COMPONENTS_BODY_BYTES,
            )
            .await?;
        let components_receive_clock = self.clock.observe()?;
        if components_receive_clock.monotonic_receive_ns()
            <= summary_receive_clock.monotonic_receive_ns()
        {
            return Err(invalid_status("source receive edges are not ordered"));
        }
        let components = parse_status_components(&components_body)?;
        let commitment = status_announcement_observation_commitment(
            self.mode,
            &summary_body,
            summary_receive_clock,
            &components_body,
            components_receive_clock,
        );
        Ok(PmStatusAnnouncementObservation {
            page: summary.page,
            active_incidents: summary.active_incidents,
            active_maintenances: summary.active_maintenances,
            components,
            summary_receive_clock,
            components_receive_clock,
            commitment,
        })
    }

    /// Verify the private production mode before the first request, then seal
    /// the ordered observation in a move-only production-origin wrapper.
    pub async fn production_ordered_announcement_observation(
        &self,
    ) -> Result<PmProductionStatusAnnouncementObservation, PmStatusAnnouncementError> {
        let production_origin = ProductionStatusOrigin::verify(self.mode)?;
        let observation = self.ordered_announcement_observation().await?;
        Ok(PmProductionStatusAnnouncementObservation::from_source(
            production_origin,
            observation,
        ))
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmStatusAnnouncementHttpRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmStatusAnnouncementHttpRole(<fixed-ordered-GET-summary-components>)")
    }
}

struct ParsedStatusSummary {
    page: PmStatusPageAnnouncement,
    active_incidents: Vec<PmStatusActiveIncident>,
    active_maintenances: Vec<PmStatusActiveMaintenance>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawStatusSummary {
    page: RawStatusPage,
    #[serde(default)]
    active_incidents: Vec<RawStatusIncident>,
    #[serde(default)]
    active_maintenances: Vec<RawStatusMaintenance>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStatusPage {
    name: String,
    url: String,
    status: RawStatusPageState,
}

#[derive(Deserialize)]
enum RawStatusPageState {
    #[serde(rename = "UP")]
    Up,
    #[serde(rename = "HASISSUES")]
    HasIssues,
    #[serde(rename = "UNDERMAINTENANCE")]
    UnderMaintenance,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawStatusIncident {
    id: String,
    name: String,
    started: String,
    status: RawStatusIncidentState,
    impact: RawStatusIncidentImpact,
    url: String,
    updated_at: String,
}

#[derive(Deserialize)]
enum RawStatusIncidentState {
    #[serde(rename = "INVESTIGATING")]
    Investigating,
    #[serde(rename = "IDENTIFIED")]
    Identified,
    #[serde(rename = "MONITORING")]
    Monitoring,
    #[serde(rename = "RESOLVED")]
    Resolved,
}

#[derive(Deserialize)]
enum RawStatusIncidentImpact {
    #[serde(rename = "NONE")]
    None,
    #[serde(rename = "MINOROUTAGE")]
    MinorOutage,
    #[serde(rename = "DEGRADEDPERFORMANCE")]
    DegradedPerformance,
    #[serde(rename = "PARTIALOUTAGE")]
    PartialOutage,
    #[serde(rename = "MAJOROUTAGE")]
    MajorOutage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawStatusMaintenance {
    id: String,
    name: String,
    start: String,
    status: RawStatusMaintenanceState,
    duration: String,
    url: String,
    updated_at: String,
}

#[derive(Deserialize)]
enum RawStatusMaintenanceState {
    #[serde(rename = "NOTSTARTEDYET")]
    NotStartedYet,
    #[serde(rename = "INPROGRESS")]
    InProgress,
    #[serde(rename = "COMPLETED")]
    Completed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStatusComponentsEnvelope {
    components: Vec<RawStatusComponent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStatusComponent {
    id: String,
    name: String,
    description: String,
    status: RawStatusComponentState,
    group: RawStatusComponentGroupField,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStatusComponentGroup {
    id: String,
    name: String,
    description: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawStatusComponentGroupField {
    Group(RawStatusComponentGroup),
    Null(()),
}

#[derive(Deserialize)]
enum RawStatusComponentState {
    #[serde(rename = "OPERATIONAL")]
    Operational,
    #[serde(rename = "UNDERMAINTENANCE")]
    UnderMaintenance,
    #[serde(rename = "DEGRADEDPERFORMANCE")]
    DegradedPerformance,
    #[serde(rename = "PARTIALOUTAGE")]
    PartialOutage,
    #[serde(rename = "MAJOROUTAGE")]
    MajorOutage,
}

fn parse_status_summary(raw: &[u8]) -> Result<ParsedStatusSummary, PmStatusAnnouncementError> {
    let parsed: RawStatusSummary =
        serde_json::from_slice(raw).map_err(|_| invalid_status("summary JSON schema mismatch"))?;
    if parsed.active_incidents.len() > MAX_PM_STATUS_ACTIVE_INCIDENTS {
        return Err(invalid_status("too many active incidents"));
    }
    if parsed.active_maintenances.len() > MAX_PM_STATUS_ACTIVE_MAINTENANCES {
        return Err(invalid_status("too many active maintenances"));
    }
    if parsed.page.name != "Polymarket" || parsed.page.url != PM_STATUS_PRODUCTION_ORIGIN {
        return Err(invalid_status("summary belongs to another status page"));
    }

    let page = PmStatusPageAnnouncement {
        name: parsed.page.name.into_boxed_str(),
        url: parsed.page.url.into_boxed_str(),
        state: map_page_state(parsed.page.status),
    };
    let mut incident_ids = HashSet::with_capacity(parsed.active_incidents.len());
    let mut active_incidents = Vec::with_capacity(parsed.active_incidents.len());
    for incident in parsed.active_incidents {
        validate_id(&incident.id)?;
        if !incident_ids.insert(incident.id.clone()) {
            return Err(invalid_status("duplicate active incident ID"));
        }
        validate_nonempty_text(&incident.name, MAX_STATUS_NAME_BYTES, "incident name")?;
        validate_utc_timestamp(&incident.started)?;
        validate_status_url(&incident.url)?;
        validate_utc_timestamp(&incident.updated_at)?;
        active_incidents.push(PmStatusActiveIncident {
            id: incident.id.into_boxed_str(),
            name: incident.name.into_boxed_str(),
            started_utc: incident.started.into_boxed_str(),
            state: map_incident_state(incident.status),
            impact: map_incident_impact(incident.impact),
            url: incident.url.into_boxed_str(),
            updated_at_utc: incident.updated_at.into_boxed_str(),
        });
    }

    let mut maintenance_ids = HashSet::with_capacity(parsed.active_maintenances.len());
    let mut active_maintenances = Vec::with_capacity(parsed.active_maintenances.len());
    for maintenance in parsed.active_maintenances {
        validate_id(&maintenance.id)?;
        if !maintenance_ids.insert(maintenance.id.clone()) {
            return Err(invalid_status("duplicate active maintenance ID"));
        }
        if incident_ids.contains(&maintenance.id) {
            return Err(invalid_status("active issue ID changes kind"));
        }
        validate_nonempty_text(&maintenance.name, MAX_STATUS_NAME_BYTES, "maintenance name")?;
        validate_utc_timestamp(&maintenance.start)?;
        validate_status_url(&maintenance.url)?;
        validate_utc_timestamp(&maintenance.updated_at)?;
        let duration_minutes = parse_duration_minutes(&maintenance.duration)?;
        active_maintenances.push(PmStatusActiveMaintenance {
            id: maintenance.id.into_boxed_str(),
            name: maintenance.name.into_boxed_str(),
            start_utc: maintenance.start.into_boxed_str(),
            state: map_maintenance_state(maintenance.status),
            duration_minutes,
            url: maintenance.url.into_boxed_str(),
            updated_at_utc: maintenance.updated_at.into_boxed_str(),
        });
    }

    Ok(ParsedStatusSummary {
        page,
        active_incidents,
        active_maintenances,
    })
}

fn parse_status_components(
    raw: &[u8],
) -> Result<Vec<PmStatusComponentAnnouncement>, PmStatusAnnouncementError> {
    let parsed: RawStatusComponentsEnvelope = serde_json::from_slice(raw)
        .map_err(|_| invalid_status("components JSON schema mismatch"))?;
    if parsed.components.len() > MAX_PM_STATUS_COMPONENTS {
        return Err(invalid_status("too many status components"));
    }

    let mut ids = HashSet::with_capacity(parsed.components.len());
    let mut components = Vec::with_capacity(parsed.components.len());
    for component in parsed.components {
        validate_id(&component.id)?;
        if !ids.insert(component.id.clone()) {
            return Err(invalid_status("duplicate component ID"));
        }
        validate_nonempty_text(&component.name, MAX_STATUS_NAME_BYTES, "component name")?;
        validate_text(
            &component.description,
            MAX_STATUS_DESCRIPTION_BYTES,
            "component description",
        )?;
        let group = match component.group {
            RawStatusComponentGroupField::Group(group) => {
                validate_id(&group.id)?;
                validate_nonempty_text(&group.name, MAX_STATUS_NAME_BYTES, "group name")?;
                validate_text(
                    &group.description,
                    MAX_STATUS_DESCRIPTION_BYTES,
                    "group description",
                )?;
                Some(PmStatusComponentGroup {
                    id: group.id.into_boxed_str(),
                    name: group.name.into_boxed_str(),
                    description: group.description.into_boxed_str(),
                })
            }
            RawStatusComponentGroupField::Null(()) => None,
        };
        components.push(PmStatusComponentAnnouncement {
            id: component.id.into_boxed_str(),
            name: component.name.into_boxed_str(),
            description: component.description.into_boxed_str(),
            state: map_component_state(component.status),
            group,
        });
    }
    validate_component_groups(&components)?;
    Ok(components)
}

fn validate_component_groups(
    components: &[PmStatusComponentAnnouncement],
) -> Result<(), PmStatusAnnouncementError> {
    let by_id: HashMap<&str, &PmStatusComponentAnnouncement> =
        components.iter().map(|row| (row.id(), row)).collect();
    for component in components {
        let Some(group) = component.group() else {
            continue;
        };
        let parent = by_id
            .get(group.id())
            .ok_or_else(|| invalid_status("component group parent is missing"))?;
        if parent.name() != group.name() || parent.description() != group.description() {
            return Err(invalid_status("component group parent details disagree"));
        }
    }
    Ok(())
}

const fn map_page_state(state: RawStatusPageState) -> PmStatusPageState {
    match state {
        RawStatusPageState::Up => PmStatusPageState::Up,
        RawStatusPageState::HasIssues => PmStatusPageState::HasIssues,
        RawStatusPageState::UnderMaintenance => PmStatusPageState::UnderMaintenance,
    }
}

const fn map_component_state(state: RawStatusComponentState) -> PmStatusComponentState {
    match state {
        RawStatusComponentState::Operational => PmStatusComponentState::Operational,
        RawStatusComponentState::UnderMaintenance => PmStatusComponentState::UnderMaintenance,
        RawStatusComponentState::DegradedPerformance => PmStatusComponentState::DegradedPerformance,
        RawStatusComponentState::PartialOutage => PmStatusComponentState::PartialOutage,
        RawStatusComponentState::MajorOutage => PmStatusComponentState::MajorOutage,
    }
}

const fn map_incident_state(state: RawStatusIncidentState) -> PmStatusIncidentState {
    match state {
        RawStatusIncidentState::Investigating => PmStatusIncidentState::Investigating,
        RawStatusIncidentState::Identified => PmStatusIncidentState::Identified,
        RawStatusIncidentState::Monitoring => PmStatusIncidentState::Monitoring,
        RawStatusIncidentState::Resolved => PmStatusIncidentState::Resolved,
    }
}

const fn map_incident_impact(impact: RawStatusIncidentImpact) -> PmStatusIncidentImpact {
    match impact {
        RawStatusIncidentImpact::None => PmStatusIncidentImpact::None,
        RawStatusIncidentImpact::MinorOutage => PmStatusIncidentImpact::MinorOutage,
        RawStatusIncidentImpact::DegradedPerformance => PmStatusIncidentImpact::DegradedPerformance,
        RawStatusIncidentImpact::PartialOutage => PmStatusIncidentImpact::PartialOutage,
        RawStatusIncidentImpact::MajorOutage => PmStatusIncidentImpact::MajorOutage,
    }
}

const fn map_maintenance_state(state: RawStatusMaintenanceState) -> PmStatusMaintenanceState {
    match state {
        RawStatusMaintenanceState::NotStartedYet => PmStatusMaintenanceState::NotStartedYet,
        RawStatusMaintenanceState::InProgress => PmStatusMaintenanceState::InProgress,
        RawStatusMaintenanceState::Completed => PmStatusMaintenanceState::Completed,
    }
}

fn validate_id(value: &str) -> Result<(), PmStatusAnnouncementError> {
    if value.is_empty()
        || value.len() > MAX_STATUS_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(invalid_status("status ID is invalid"));
    }
    Ok(())
}

fn validate_nonempty_text(
    value: &str,
    maximum_bytes: usize,
    field: &'static str,
) -> Result<(), PmStatusAnnouncementError> {
    validate_text(value, maximum_bytes, field)?;
    if value.trim().is_empty() {
        return Err(invalid_status(field));
    }
    Ok(())
}

fn validate_text(
    value: &str,
    maximum_bytes: usize,
    field: &'static str,
) -> Result<(), PmStatusAnnouncementError> {
    if value.len() > maximum_bytes
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(invalid_status(field));
    }
    Ok(())
}

fn validate_status_url(value: &str) -> Result<(), PmStatusAnnouncementError> {
    if value.is_empty() || value.len() > MAX_STATUS_URL_BYTES {
        return Err(invalid_status("status notice URL is invalid"));
    }
    let parsed = Url::parse(value).map_err(|_| invalid_status("status notice URL is invalid"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("status.polymarket.com")
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() == "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid_status("status notice URL is invalid"));
    }
    Ok(())
}

fn validate_utc_timestamp(value: &str) -> Result<(), PmStatusAnnouncementError> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.len() > MAX_STATUS_TIMESTAMP_BYTES
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes.last() != Some(&b'Z')
    {
        return Err(invalid_status("status timestamp is not canonical UTC"));
    }
    let year = decimal_digits(&bytes[0..4])?;
    let month = decimal_digits(&bytes[5..7])?;
    let day = decimal_digits(&bytes[8..10])?;
    let hour = decimal_digits(&bytes[11..13])?;
    let minute = decimal_digits(&bytes[14..16])?;
    let second = decimal_digits(&bytes[17..19])?;
    let fractional = &bytes[19..bytes.len() - 1];
    if !fractional.is_empty()
        && (fractional[0] != b'.'
            || fractional.len() == 1
            || fractional.len() > 10
            || !fractional[1..].iter().all(u8::is_ascii_digit))
    {
        return Err(invalid_status("status timestamp is not canonical UTC"));
    }
    let maximum_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > maximum_day || hour > 23 || minute > 59 || second > 59 {
        return Err(invalid_status("status timestamp is not canonical UTC"));
    }
    Ok(())
}

fn decimal_digits(bytes: &[u8]) -> Result<u32, PmStatusAnnouncementError> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(invalid_status("status timestamp is not canonical UTC"));
    }
    bytes.iter().try_fold(0_u32, |value, byte| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u32::from(*byte - b'0')))
            .ok_or_else(|| invalid_status("status timestamp is not canonical UTC"))
    })
}

fn parse_duration_minutes(value: &str) -> Result<u32, PmStatusAnnouncementError> {
    if value.is_empty()
        || value.len() > 10
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(invalid_status("maintenance duration is invalid"));
    }
    let duration = value
        .parse::<u32>()
        .map_err(|_| invalid_status("maintenance duration is invalid"))?;
    if duration == 0 {
        return Err(invalid_status("maintenance duration is invalid"));
    }
    Ok(duration)
}

const fn invalid_status(reason: &'static str) -> PmStatusAnnouncementError {
    PmStatusAnnouncementError::InvalidResponse(reason)
}

fn status_announcement_observation_commitment(
    mode: OriginMode,
    summary_body: &[u8],
    summary_receive_clock: PmHttpReceiveClock,
    components_body: &[u8],
    components_receive_clock: PmHttpReceiveClock,
) -> PmStatusAnnouncementObservationCommitment {
    let mut digest = Sha256::new();
    encode_status_bytes(
        &mut digest,
        STATUS_ANNOUNCEMENT_OBSERVATION_COMMITMENT_DOMAIN,
    );
    encode_status_bytes(&mut digest, STATUS_ANNOUNCEMENT_SCHEMA);
    encode_status_bytes(&mut digest, origin_mode_name(mode));
    encode_status_bytes(&mut digest, PM_STATUS_PRODUCTION_ORIGIN.as_bytes());
    encode_status_bytes(&mut digest, b"GET");
    encode_status_bytes(&mut digest, b"/v3/summary.json");
    digest.update(summary_receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(summary_receive_clock.monotonic_receive_ns().to_be_bytes());
    encode_status_bytes(&mut digest, summary_body);
    encode_status_bytes(&mut digest, b"GET");
    encode_status_bytes(&mut digest, b"/v3/components.json");
    digest.update(
        components_receive_clock
            .local_wall_receive_ns()
            .to_be_bytes(),
    );
    digest.update(
        components_receive_clock
            .monotonic_receive_ns()
            .to_be_bytes(),
    );
    encode_status_bytes(&mut digest, components_body);
    PmStatusAnnouncementObservationCommitment::from_source_bytes(digest.finalize().into())
}

fn encode_status_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded status commitment field length fits u64")
            .to_be_bytes(),
    );
    digest.update(value);
}

const fn origin_mode_name(mode: OriginMode) -> &'static [u8] {
    match mode {
        OriginMode::Production => b"production",
        #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
        OriginMode::LocalEvidence => b"local-evidence",
    }
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
        task::JoinHandle,
        time::sleep,
    };

    use super::*;

    const EMPTY_SUMMARY: &[u8] = br#"{
        "page": {
            "name": "Polymarket",
            "url": "https://status.polymarket.com",
            "status": "UP"
        }
    }"#;
    const EMPTY_COMPONENTS: &[u8] = br#"{
        "components": [
            {
                "id": "predictions1",
                "name": "Predictions",
                "description": "",
                "status": "OPERATIONAL",
                "group": null
            },
            {
                "id": "clobapi1",
                "name": "  Trading API (CLOB)",
                "description": "",
                "status": "OPERATIONAL",
                "group": {
                    "id": "predictions1",
                    "name": "Predictions",
                    "description": ""
                }
            }
        ]
    }"#;
    const ACTIVE_SUMMARY: &[u8] = br#"{
        "page": {
            "name": "Polymarket",
            "url": "https://status.polymarket.com",
            "status": "HASISSUES"
        },
        "activeIncidents": [
            {
                "id": "incident1",
                "name": "Trading API degraded",
                "started": "2026-08-09T17:00:00Z",
                "status": "INVESTIGATING",
                "impact": "MAJOROUTAGE",
                "url": "https://status.polymarket.com/default/incident1",
                "updatedAt": "2026-08-09T17:01:00.123Z"
            }
        ],
        "activeMaintenances": [
            {
                "id": "maintenance1",
                "name": "Trading maintenance",
                "start": "2026-08-09T18:00:00Z",
                "status": "NOTSTARTEDYET",
                "duration": "60",
                "url": "https://status.polymarket.com/default/maintenance1",
                "updatedAt": "2026-08-09T17:02:00Z"
            }
        ]
    }"#;

    struct MockResponse {
        status: u16,
        body: Vec<u8>,
        delay: Duration,
        location: Option<&'static str>,
        content_length: Option<usize>,
    }

    impl MockResponse {
        fn ok(body: impl Into<Vec<u8>>) -> Self {
            let body = body.into();
            Self {
                status: 200,
                content_length: Some(body.len()),
                body,
                delay: Duration::ZERO,
                location: None,
            }
        }
    }

    async fn mock_server(
        responses: Vec<MockResponse>,
    ) -> (String, mpsc::UnboundedReceiver<String>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (requests_tx, requests_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    raw.extend_from_slice(&chunk[..read]);
                    if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests_tx.send(String::from_utf8(raw).unwrap()).unwrap();
                sleep(response.delay).await;
                let reason = match response.status {
                    200 => "OK",
                    302 => "Found",
                    _ => "Mock",
                };
                let mut headers = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
                    response.status, reason,
                );
                if let Some(length) = response.content_length {
                    headers.push_str(&format!("Content-Length: {length}\r\n"));
                }
                if let Some(location) = response.location {
                    headers.push_str(&format!("Location: {location}\r\n"));
                }
                headers.push_str("\r\n");
                if stream.write_all(headers.as_bytes()).await.is_ok() {
                    let _ = stream.write_all(&response.body).await;
                }
            }
        });
        (format!("http://{address}"), requests_rx, task)
    }

    fn local_role(origin: &str, request_timeout: Duration) -> PmStatusAnnouncementHttpRole {
        PmStatusAnnouncementHttpRole::read_only_evidence(
            origin,
            Duration::from_millis(100),
            request_timeout,
        )
        .unwrap()
    }

    fn assert_invalid_summary(raw: impl AsRef<[u8]>) {
        assert!(matches!(
            parse_status_summary(raw.as_ref()),
            Err(PmStatusAnnouncementError::InvalidResponse(_)),
        ));
    }

    fn assert_invalid_components(raw: impl AsRef<[u8]>) {
        assert!(matches!(
            parse_status_components(raw.as_ref()),
            Err(PmStatusAnnouncementError::InvalidResponse(_)),
        ));
    }

    #[tokio::test]
    async fn fixed_ordered_cut_retains_all_typed_current_announcement_rows() {
        let (origin, mut requests, server) = mock_server(vec![
            MockResponse::ok(ACTIVE_SUMMARY),
            MockResponse::ok(EMPTY_COMPONENTS),
        ])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        let observation = role.ordered_announcement_observation().await.unwrap();

        assert_eq!(observation.page().name(), "Polymarket");
        assert_eq!(observation.page().url(), PM_STATUS_PRODUCTION_ORIGIN);
        assert_eq!(observation.page().state(), PmStatusPageState::HasIssues);
        assert_eq!(observation.active_incidents().len(), 1);
        let incident = &observation.active_incidents()[0];
        assert_eq!(incident.id(), "incident1");
        assert_eq!(incident.name(), "Trading API degraded");
        assert_eq!(incident.started_utc(), "2026-08-09T17:00:00Z");
        assert_eq!(incident.state(), PmStatusIncidentState::Investigating);
        assert_eq!(incident.impact(), PmStatusIncidentImpact::MajorOutage);
        assert_eq!(
            incident.url(),
            "https://status.polymarket.com/default/incident1",
        );
        assert_eq!(incident.updated_at_utc(), "2026-08-09T17:01:00.123Z");
        assert_eq!(observation.active_maintenances().len(), 1);
        let maintenance = &observation.active_maintenances()[0];
        assert_eq!(maintenance.id(), "maintenance1");
        assert_eq!(maintenance.name(), "Trading maintenance");
        assert_eq!(maintenance.start_utc(), "2026-08-09T18:00:00Z");
        assert_eq!(maintenance.state(), PmStatusMaintenanceState::NotStartedYet,);
        assert_eq!(maintenance.duration_minutes(), 60);
        assert_eq!(
            maintenance.url(),
            "https://status.polymarket.com/default/maintenance1",
        );
        assert_eq!(maintenance.updated_at_utc(), "2026-08-09T17:02:00Z");
        assert_eq!(observation.components().len(), 2);
        assert_eq!(
            observation.components()[1].state(),
            PmStatusComponentState::Operational,
        );
        let group = observation.components()[1].group().unwrap();
        assert_eq!(group.id(), "predictions1");
        assert_eq!(group.name(), "Predictions");
        assert_eq!(group.description(), "");
        assert!(
            observation
                .components_receive_clock()
                .monotonic_receive_ns()
                > observation.summary_receive_clock().monotonic_receive_ns()
        );
        assert_ne!(observation.commitment().bytes(), [0; 32]);
        assert!(!role.production_order_entry_authorized());
        assert_eq!(
            format!("{observation:?}"),
            "PmStatusAnnouncementObservation(<ordered-summary-components; announcement-only; sealed>)",
        );

        let summary_request = requests.recv().await.unwrap();
        let components_request = requests.recv().await.unwrap();
        assert!(summary_request.starts_with("GET /v3/summary.json HTTP/1.1\r\n"));
        assert!(components_request.starts_with("GET /v3/components.json HTTP/1.1\r\n"));
        for request in [summary_request, components_request] {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("accept: application/json\r\n")
            );
        }
        server.await.unwrap();
    }

    #[test]
    fn omitted_empty_summary_issue_arrays_match_current_production_shape() {
        let summary = parse_status_summary(EMPTY_SUMMARY).unwrap();
        assert_eq!(summary.page.state(), PmStatusPageState::Up);
        assert!(summary.active_incidents.is_empty());
        assert!(summary.active_maintenances.is_empty());
        let components = parse_status_components(EMPTY_COMPONENTS).unwrap();
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn stale_documentation_bare_array_shape_and_unreviewed_issue_fields_are_rejected() {
        assert_invalid_components(
            br#"[{"id":"clobapi1","name":"CLOB","description":"","status":"OPERATIONAL","isParent":false,"children":[]}]"#,
        );
        assert_invalid_components(
            br#"{"components":[{"id":"clobapi1","name":"CLOB","description":"","status":"OPERATIONAL","group":null,"activeIncidents":[]}]}"#,
        );
        assert_invalid_components(
            br#"{"components":[{"id":"clobapi1","name":"CLOB","description":"","status":"OPERATIONAL","group":null,"activeMaintenances":[]}]}"#,
        );
    }

    #[test]
    fn summary_unknown_duplicate_missing_type_and_enum_drift_fail_closed() {
        for invalid in [
            br#"{}"#.as_slice(),
            br#"{"page":{"name":"Polymarket","url":"https://status.polymarket.com","status":"UP","unknown":true}}"#.as_slice(),
            br#"{"page":{"name":"Polymarket","name":"Polymarket","url":"https://status.polymarket.com","status":"UP"}}"#.as_slice(),
            br#"{"page":{"name":"Polymarket","url":"https://status.polymarket.com","status":"UNKNOWN"}}"#.as_slice(),
            br#"{"page":{"name":"Polymarket","url":"https://status.polymarket.com","status":"UP"},"activeIncidents":{}}"#.as_slice(),
            br#"{"page":{"name":"Another","url":"https://status.polymarket.com","status":"UP"}}"#.as_slice(),
            br#"{"page":{"name":"Polymarket","url":"https://evil.example","status":"UP"}}"#.as_slice(),
        ] {
            assert_invalid_summary(invalid);
        }
    }

    #[test]
    fn active_issue_rows_reject_missing_unknown_duplicate_ids_and_bad_scalars() {
        let incident = |fields: &str| {
            format!(
                r#"{{"page":{{"name":"Polymarket","url":"https://status.polymarket.com","status":"HASISSUES"}},"activeIncidents":[{{{fields}}}]}}"#,
            )
        };
        for invalid in [
            incident(
                r#""id":"incident1","name":"Issue","started":"2026-08-09T17:00:00Z","status":"INVESTIGATING","impact":"MAJOROUTAGE","url":"https://status.polymarket.com/default/incident1""#,
            ),
            incident(
                r#""id":"incident1","name":"Issue","started":"2026-08-09 17:00:00Z","status":"INVESTIGATING","impact":"MAJOROUTAGE","url":"https://status.polymarket.com/default/incident1","updatedAt":"2026-08-09T17:01:00Z""#,
            ),
            incident(
                r#""id":"incident1","name":"Issue","started":"2026-08-09T17:00:00Z","status":"NEW","impact":"MAJOROUTAGE","url":"https://status.polymarket.com/default/incident1","updatedAt":"2026-08-09T17:01:00Z""#,
            ),
            incident(
                r#""id":"INCIDENT-1","name":"Issue","started":"2026-08-09T17:00:00Z","status":"INVESTIGATING","impact":"MAJOROUTAGE","url":"https://status.polymarket.com/default/incident1","updatedAt":"2026-08-09T17:01:00Z""#,
            ),
            incident(
                r#""id":"incident1","name":"Issue","started":"2026-08-09T17:00:00Z","status":"INVESTIGATING","impact":"CATASTROPHIC","url":"https://status.polymarket.com/default/incident1","updatedAt":"2026-08-09T17:01:00Z""#,
            ),
            incident(
                r#""id":"incident1","name":"Issue","started":"2026-08-09T17:00:00Z","status":"INVESTIGATING","impact":"MAJOROUTAGE","url":"http://status.polymarket.com/default/incident1","updatedAt":"2026-08-09T17:01:00Z""#,
            ),
            incident(
                r#""id":"incident1","name":"Issue","started":"2026-08-09T17:00:00Z","status":"INVESTIGATING","impact":"MAJOROUTAGE","url":"https://status.polymarket.com/default/incident1","updatedAt":"2026-08-09T17:01:00Z","unknown":true"#,
            ),
        ] {
            assert_invalid_summary(invalid);
        }

        let duplicate = format!(
            r#"{{"page":{{"name":"Polymarket","url":"https://status.polymarket.com","status":"HASISSUES"}},"activeIncidents":[{row},{row}]}}"#,
            row = r#"{"id":"incident1","name":"Issue","started":"2026-08-09T17:00:00Z","status":"INVESTIGATING","impact":"MAJOROUTAGE","url":"https://status.polymarket.com/default/incident1","updatedAt":"2026-08-09T17:01:00Z"}"#,
        );
        assert_invalid_summary(duplicate);

        for invalid in [
            br#"{"page":{"name":"Polymarket","url":"https://status.polymarket.com","status":"UNDERMAINTENANCE"},"activeMaintenances":[{"id":"maintenance1","name":"Work","start":"2026-08-09T18:00:00Z","status":"INPROGRESS","duration":"0","url":"https://status.polymarket.com/default/maintenance1","updatedAt":"2026-08-09T17:00:00Z"}]}"#.as_slice(),
            br#"{"page":{"name":"Polymarket","url":"https://status.polymarket.com","status":"UNDERMAINTENANCE"},"activeMaintenances":[{"id":"maintenance1","name":"Work","start":"2026-08-09T18:00:00Z","status":"INPROGRESS","duration":60,"url":"https://status.polymarket.com/default/maintenance1","updatedAt":"2026-08-09T17:00:00Z"}]}"#.as_slice(),
            br#"{"page":{"name":"Polymarket","url":"https://status.polymarket.com","status":"UNDERMAINTENANCE"},"activeMaintenances":[{"id":"maintenance1","name":"Work","start":"2026-02-30T18:00:00Z","status":"INPROGRESS","duration":"60","url":"https://status.polymarket.com/default/maintenance1","updatedAt":"2026-08-09T17:00:00Z"}]}"#.as_slice(),
        ] {
            assert_invalid_summary(invalid);
        }
    }

    #[test]
    fn component_rows_reject_unknown_duplicate_missing_group_and_inconsistent_parent() {
        for invalid in [
            br#"{}"#.as_slice(),
            br#"{"components":[],"unknown":true}"#.as_slice(),
            br#"{"components":{},"components":[]}"#.as_slice(),
            br#"{"components":[{"id":"clobapi1","name":"CLOB","description":"","status":"OPERATIONAL"}]}"#.as_slice(),
            br#"{"components":[{"id":"clobapi1","name":"CLOB","description":"","status":"UNKNOWN","group":null}]}"#.as_slice(),
            br#"{"components":[{"id":"clobapi1","name":"CLOB","description":"","status":"OPERATIONAL","group":{"id":"missing1","name":"Missing","description":""}}]}"#.as_slice(),
            br#"{"components":[{"id":"parent1","name":"Parent","description":"","status":"OPERATIONAL","group":null},{"id":"child1","name":"Child","description":"","status":"OPERATIONAL","group":{"id":"parent1","name":"Changed","description":""}}]}"#.as_slice(),
            br#"{"components":[{"id":"clobapi1","name":"CLOB","description":"","status":"OPERATIONAL","group":null},{"id":"clobapi1","name":"Again","description":"","status":"OPERATIONAL","group":null}]}"#.as_slice(),
        ] {
            assert_invalid_components(invalid);
        }
    }

    #[test]
    fn row_count_and_string_bounds_fail_closed() {
        let incident_row = r#"{"id":"incident1","name":"Issue","started":"2026-08-09T17:00:00Z","status":"INVESTIGATING","impact":"MAJOROUTAGE","url":"https://status.polymarket.com/default/incident1","updatedAt":"2026-08-09T17:01:00Z"}"#;
        let too_many_incidents = format!(
            r#"{{"page":{{"name":"Polymarket","url":"https://status.polymarket.com","status":"HASISSUES"}},"activeIncidents":[{}]}}"#,
            std::iter::repeat_n(incident_row, MAX_PM_STATUS_ACTIVE_INCIDENTS + 1)
                .collect::<Vec<_>>()
                .join(","),
        );
        assert_invalid_summary(too_many_incidents);

        let long_name = "x".repeat(MAX_STATUS_NAME_BYTES + 1);
        let long_component = format!(
            r#"{{"components":[{{"id":"component1","name":"{long_name}","description":"","status":"OPERATIONAL","group":null}}]}}"#,
        );
        assert_invalid_components(long_component);

        let components = (0..=MAX_PM_STATUS_COMPONENTS)
            .map(|index| {
                format!(
                    r#"{{"id":"component{index}","name":"Component {index}","description":"","status":"OPERATIONAL","group":null}}"#,
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        assert_invalid_components(format!(r#"{{"components":[{components}]}}"#));
    }

    #[tokio::test]
    async fn summary_and_components_declared_body_bounds_fail_before_parse() {
        let summary_oversize = vec![b'x'; MAX_PM_STATUS_SUMMARY_BODY_BYTES + 1];
        let (origin, _requests, server) =
            mock_server(vec![MockResponse::ok(summary_oversize)]).await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert!(matches!(
            role.ordered_announcement_observation().await,
            Err(PmStatusAnnouncementError::Http(
                PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: MAX_PM_STATUS_SUMMARY_BODY_BYTES
                }
            )),
        ));
        server.await.unwrap();

        let components_oversize = vec![b'x'; MAX_PM_STATUS_COMPONENTS_BODY_BYTES + 1];
        let (origin, _requests, server) = mock_server(vec![
            MockResponse::ok(EMPTY_SUMMARY),
            MockResponse::ok(components_oversize),
        ])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert!(matches!(
            role.ordered_announcement_observation().await,
            Err(PmStatusAnnouncementError::Http(
                PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: MAX_PM_STATUS_COMPONENTS_BODY_BYTES
                }
            )),
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn streamed_body_bound_is_enforced_without_content_length() {
        let (origin, _requests, server) = mock_server(vec![MockResponse {
            status: 200,
            body: vec![b'x'; MAX_PM_STATUS_SUMMARY_BODY_BYTES + 1],
            delay: Duration::ZERO,
            location: None,
            content_length: None,
        }])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert!(matches!(
            role.ordered_announcement_observation().await,
            Err(PmStatusAnnouncementError::Http(
                PmLiveAdapterError::ResponseBodyTooLarge {
                    limit: MAX_PM_STATUS_SUMMARY_BODY_BYTES
                }
            )),
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn summary_failure_prevents_the_components_request() {
        let (origin, mut requests, server) = mock_server(vec![MockResponse {
            status: 302,
            body: Vec::new(),
            delay: Duration::ZERO,
            location: Some("/v3/summary.json"),
            content_length: Some(0),
        }])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert!(matches!(
            role.ordered_announcement_observation().await,
            Err(PmStatusAnnouncementError::Http(
                PmLiveAdapterError::Redirect { status: 302 }
            )),
        ));
        assert!(
            requests
                .recv()
                .await
                .unwrap()
                .starts_with("GET /v3/summary.json")
        );
        assert!(requests.try_recv().is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn status_and_timeout_fail_closed_without_retry() {
        let (origin, mut requests, server) = mock_server(vec![MockResponse {
            status: 503,
            body: Vec::new(),
            delay: Duration::ZERO,
            location: None,
            content_length: Some(0),
        }])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        assert!(matches!(
            role.ordered_announcement_observation().await,
            Err(PmStatusAnnouncementError::Http(
                PmLiveAdapterError::UnexpectedStatus { status: 503 }
            )),
        ));
        assert!(requests.recv().await.is_some());
        assert!(requests.try_recv().is_err());
        server.await.unwrap();

        let (origin, _requests, server) = mock_server(vec![MockResponse {
            status: 200,
            body: EMPTY_SUMMARY.to_vec(),
            delay: Duration::from_millis(100),
            location: None,
            content_length: Some(EMPTY_SUMMARY.len()),
        }])
        .await;
        let role = local_role(&origin, Duration::from_millis(20));
        assert!(matches!(
            role.ordered_announcement_observation().await,
            Err(PmStatusAnnouncementError::Http(
                PmLiveAdapterError::RequestTimeout
            )),
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn local_evidence_cannot_issue_production_proof_and_checks_before_io() {
        let role = local_role("http://127.0.0.1:9", Duration::from_millis(50));
        assert!(matches!(
            role.production_ordered_announcement_observation().await,
            Err(PmStatusAnnouncementError::Http(
                PmLiveAdapterError::InvalidConfiguration(
                    "production status announcement observation requires the fixed production origin"
                )
            )),
        ));
    }

    #[test]
    fn production_origin_proof_accepts_only_production_mode() {
        assert!(ProductionStatusOrigin::verify(OriginMode::Production).is_ok());
        assert!(matches!(
            ProductionStatusOrigin::verify(OriginMode::LocalEvidence),
            Err(PmLiveAdapterError::InvalidConfiguration(_)),
        ));
    }

    #[tokio::test]
    async fn both_bodies_and_receive_edges_are_bound_by_one_commitment() {
        let (origin, _requests, server) = mock_server(vec![
            MockResponse::ok(EMPTY_SUMMARY),
            MockResponse::ok(EMPTY_COMPONENTS),
            MockResponse::ok(EMPTY_SUMMARY),
            MockResponse::ok(EMPTY_COMPONENTS),
        ])
        .await;
        let role = local_role(&origin, Duration::from_secs(1));
        let first = role.ordered_announcement_observation().await.unwrap();
        let second = role.ordered_announcement_observation().await.unwrap();
        assert_ne!(
            first.summary_receive_clock(),
            second.summary_receive_clock()
        );
        assert_ne!(
            first.components_receive_clock(),
            second.components_receive_clock()
        );
        assert_ne!(first.commitment(), second.commitment());

        let production =
            PmProductionStatusAnnouncementObservation::from_source(ProductionStatusOrigin, first);
        assert_eq!(production.page().state(), PmStatusPageState::Up);
        assert!(production.active_incidents().is_empty());
        assert!(production.active_maintenances().is_empty());
        assert_eq!(production.components().len(), 2);
        assert_ne!(production.commitment().bytes(), [0; 32]);
        assert!(production.summary_receive_clock().local_wall_receive_ns() > 0);
        assert!(
            production
                .components_receive_clock()
                .local_wall_receive_ns()
                > 0
        );
        assert_eq!(
            format!("{production:?}"),
            "PmProductionStatusAnnouncementObservation(<production-origin; announcement-only; sealed>)",
        );
        server.await.unwrap();
    }
}
