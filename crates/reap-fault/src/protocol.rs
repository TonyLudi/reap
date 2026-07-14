use std::collections::BTreeMap;
use std::path::PathBuf;

use reap_core::PINNED_JAVA_REVISION;
use serde::{Deserialize, Serialize};

use crate::config::FaultProxyConfigEvidence;

pub const CONTROL_FORMAT_VERSION: u32 = 1;
pub const INJECTOR_EVIDENCE_FORMAT_VERSION: u32 = 1;
pub const RUN_REPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum FaultProxyCommand {
    Status,
    DisconnectWebsockets {
        command_id: String,
        evidence_file: String,
        target: WebSocketTarget,
        connections: usize,
    },
    ArmRestResponse {
        command_id: String,
        evidence_file: String,
        matcher: RestRequestMatcher,
        response: InjectedHttpResponse,
        times: u32,
    },
    ArmWebsocketDrop {
        command_id: String,
        evidence_file: String,
        target: WebSocketTarget,
        direction: WebSocketDirection,
        matcher: WebSocketFrameMatcher,
        frames: u32,
    },
    Shutdown {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketTarget {
    Public,
    Private,
    Order,
}

impl WebSocketTarget {
    pub const fn expected_path(self) -> &'static str {
        match self {
            Self::Public => "/ws/v5/public",
            Self::Private | Self::Order => "/ws/v5/private",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketDirection {
    ClientToExchange,
    ExchangeToClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketFrameKind {
    Text,
    Binary,
    Ping,
    Pong,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSocketFrameMatcher {
    pub kind: WebSocketFrameKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<WebSocketJsonMatcher>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSocketJsonMatcher {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

impl WebSocketJsonMatcher {
    pub fn is_empty(&self) -> bool {
        self.op.is_none()
            && self.event.is_none()
            && self.channel.is_none()
            && self.instrument_type.is_none()
            && self.symbol.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestRequestMatcher {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InjectedHttpResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProxyControlResponse {
    pub format_version: u32,
    pub accepted: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<FaultProxyStatus>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProxyStatus {
    pub proxy_session_id: String,
    pub rest_requests_forwarded: u64,
    pub rest_responses_injected: u64,
    pub websocket_connections_accepted: u64,
    pub websocket_connections_active: BTreeMap<String, u64>,
    pub websocket_frames_forwarded: u64,
    pub websocket_frames_dropped: u64,
    pub websocket_disconnects_injected: u64,
    pub pending_rest_faults: usize,
    pub pending_websocket_faults: usize,
    pub completed_faults: u64,
    pub proxy_errors: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultInjectorEvidence {
    pub format_version: u32,
    pub proxy_session_id: String,
    pub proxy_config_fingerprint: String,
    pub java_reference_revision: String,
    pub command_id: String,
    pub command: FaultCommandSummary,
    pub armed_at_ms: u64,
    pub completed_at_ms: u64,
    pub effects: Vec<FaultEffect>,
    pub passed: bool,
}

impl FaultInjectorEvidence {
    pub(crate) fn new(
        context: FaultEvidenceContext,
        command_id: String,
        command: FaultCommandSummary,
        effects: Vec<FaultEffect>,
        passed: bool,
    ) -> Self {
        Self {
            format_version: INJECTOR_EVIDENCE_FORMAT_VERSION,
            proxy_session_id: context.proxy_session_id,
            proxy_config_fingerprint: context.proxy_config_fingerprint,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            command_id,
            command,
            armed_at_ms: context.armed_at_ms,
            completed_at_ms: context.completed_at_ms,
            passed,
            effects,
        }
    }
}

pub(crate) struct FaultEvidenceContext {
    pub proxy_session_id: String,
    pub proxy_config_fingerprint: String,
    pub armed_at_ms: u64,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FaultCommandSummary {
    DisconnectWebsockets {
        target: WebSocketTarget,
        connections: usize,
    },
    RestResponse {
        matcher: RestRequestMatcher,
        status: u16,
        response_headers: BTreeMap<String, String>,
        response_body_bytes: u64,
        response_body_sha256: String,
        times: u32,
    },
    WebsocketDrop {
        target: WebSocketTarget,
        direction: WebSocketDirection,
        matcher: WebSocketFrameMatcher,
        frames: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FaultEffect {
    WebsocketDisconnected {
        sequence: u32,
        applied_at_ms: u64,
        connection_id: u64,
        target: WebSocketTarget,
    },
    RestResponseInjected {
        sequence: u32,
        applied_at_ms: u64,
        method: String,
        path: String,
        query_sha256: String,
    },
    WebsocketFrameDropped {
        sequence: u32,
        applied_at_ms: u64,
        connection_id: u64,
        target: WebSocketTarget,
        direction: WebSocketDirection,
        frame_kind: WebSocketFrameKind,
        frame_bytes: u64,
        frame_sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProxyRunReport {
    pub format_version: u32,
    pub proxy_session_id: String,
    pub config: FaultProxyConfigEvidence,
    pub java_reference_revision: String,
    pub started_at_ms: u64,
    pub stopped_at_ms: u64,
    pub elapsed_ms: u64,
    pub stop_reason: String,
    pub status: FaultProxyStatus,
    pub clean_shutdown: bool,
}

impl FaultProxyRunReport {
    pub fn java_revision() -> String {
        PINNED_JAVA_REVISION.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::FaultProxyCommand;

    #[test]
    fn checked_in_fault_commands_follow_the_strict_protocol() {
        for (name, bytes) in [
            (
                "status",
                include_bytes!("../../../examples/faults/status.json").as_slice(),
            ),
            (
                "shutdown",
                include_bytes!("../../../examples/faults/shutdown.json").as_slice(),
            ),
            (
                "public reconnect",
                include_bytes!("../../../examples/faults/public-reconnect.json").as_slice(),
            ),
            (
                "private reconnect",
                include_bytes!("../../../examples/faults/private-reconnect.json").as_slice(),
            ),
            (
                "order reconnect",
                include_bytes!("../../../examples/faults/order-transport-reconnect.json")
                    .as_slice(),
            ),
            (
                "ambiguous submit",
                include_bytes!("../../../examples/faults/ambiguous-submit.json").as_slice(),
            ),
            (
                "ambiguous cancel",
                include_bytes!("../../../examples/faults/ambiguous-cancel.json").as_slice(),
            ),
            (
                "exchange clock failure",
                include_bytes!("../../../examples/faults/exchange-clock-failure.json").as_slice(),
            ),
            (
                "fill convergence timeout",
                include_bytes!("../../../examples/faults/fill-convergence-timeout.json").as_slice(),
            ),
            (
                "order convergence timeout",
                include_bytes!("../../../examples/faults/order-convergence-timeout.json")
                    .as_slice(),
            ),
            (
                "deadman heartbeat failure",
                include_bytes!("../../../examples/faults/deadman-heartbeat-failure.json")
                    .as_slice(),
            ),
            (
                "exchange status failure",
                include_bytes!("../../../examples/faults/exchange-status-failure.json").as_slice(),
            ),
            (
                "exchange instrument failure",
                include_bytes!("../../../examples/faults/exchange-instrument-failure.json")
                    .as_slice(),
            ),
            (
                "exchange fee failure",
                include_bytes!("../../../examples/faults/exchange-fee-failure.json").as_slice(),
            ),
            (
                "account config failure",
                include_bytes!("../../../examples/faults/account-config-failure.json").as_slice(),
            ),
        ] {
            serde_json::from_slice::<FaultProxyCommand>(bytes)
                .unwrap_or_else(|error| panic!("invalid {name} command: {error}"));
        }
    }
}
