use reap_core::RawEnvelope;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

const MAX_PUBLIC_FRAME_BYTES: usize = 16 * 1024;
const MAX_REFERENCE_VALUES_PER_FRAME: usize = 16;

#[derive(Debug)]
pub(crate) enum DecodedPublicFrame {
    Heartbeat,
    Acknowledgement {
        code: Option<Value>,
        arg: Option<WireArg>,
    },
    ConnectionCount {
        channel: String,
        connection_count: u64,
        connection_id: String,
    },
    RejectedControl {
        event: &'static str,
        code: Option<Value>,
        message: String,
    },
    StateChangingControl {
        event: &'static str,
    },
    Data {
        arg: WireArg,
        values: Vec<WireIndexTicker>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireArg {
    pub(crate) channel: String,
    #[serde(default, rename = "instId")]
    pub(crate) instrument: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireIndexTicker {
    #[serde(default, rename = "instId")]
    pub(crate) instrument: Option<String>,
    #[serde(rename = "idxPx")]
    pub(crate) index_price: String,
    pub(crate) ts: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAcknowledgement {
    event: String,
    #[serde(default)]
    code: Option<Value>,
    #[serde(default, rename = "msg")]
    message: Option<String>,
    #[serde(default)]
    arg: Option<WireArg>,
    #[serde(default, rename = "connId")]
    connection_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireConnectionCount {
    event: String,
    channel: String,
    #[serde(rename = "connCount")]
    connection_count: String,
    #[serde(rename = "connId")]
    connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireServerError {
    event: String,
    #[serde(default)]
    code: Option<Value>,
    #[serde(default, rename = "msg")]
    message: Option<String>,
    #[serde(default, rename = "connId")]
    connection_id: Option<String>,
    #[serde(default)]
    arg: Option<WireArg>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default, rename = "connCount")]
    connection_count: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNotice {
    event: String,
    #[serde(default)]
    code: Option<Value>,
    #[serde(default, rename = "msg")]
    message: Option<String>,
    #[serde(default, rename = "connId")]
    connection_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDataFrame {
    arg: WireArg,
    #[serde(default)]
    data: Vec<WireIndexTicker>,
}

pub(crate) fn decode_public_frame(
    envelope: &RawEnvelope,
) -> Result<DecodedPublicFrame, PublicWireError> {
    let payload = envelope.payload.as_str();
    if payload.len() > MAX_PUBLIC_FRAME_BYTES {
        return Err(PublicWireError::FrameTooLarge);
    }
    if payload == "pong" {
        return Ok(DecodedPublicFrame::Heartbeat);
    }
    let value: Value =
        serde_json::from_str(payload).map_err(|error| PublicWireError::Malformed {
            reason: error.to_string(),
        })?;
    let object = value.as_object().ok_or(PublicWireError::NonObject)?;
    let explicit_null_code = object.get("code").is_some_and(Value::is_null);
    if let Some(event) = object.get("event") {
        let event = event
            .as_str()
            .ok_or(PublicWireError::ControlEventNotString)?
            .to_string();
        return match event.as_str() {
            "subscribe" => {
                let frame: WireAcknowledgement = decode_shape(value)?;
                validate_event(&frame.event, "subscribe")?;
                validate_optional_bounded("msg", frame.message.as_deref())?;
                validate_optional_bounded("connId", frame.connection_id.as_deref())?;
                Ok(DecodedPublicFrame::Acknowledgement {
                    code: if explicit_null_code {
                        Some(Value::Null)
                    } else {
                        frame.code
                    },
                    arg: frame.arg,
                })
            }
            "channel-conn-count" => {
                let frame: WireConnectionCount = decode_shape(value)?;
                validate_event(&frame.event, "channel-conn-count")?;
                validate_required_bounded("channel", &frame.channel)?;
                validate_required_bounded("connId", &frame.connection_id)?;
                let connection_count = frame
                    .connection_count
                    .parse::<u64>()
                    .map_err(|_| PublicWireError::InvalidControlField { field: "connCount" })?;
                if connection_count == 0 {
                    return Err(PublicWireError::InvalidControlField { field: "connCount" });
                }
                Ok(DecodedPublicFrame::ConnectionCount {
                    channel: frame.channel,
                    connection_count,
                    connection_id: frame.connection_id,
                })
            }
            "error" => {
                let frame: WireServerError = decode_shape(value)?;
                validate_event(&frame.event, "error")?;
                validate_optional_bounded("msg", frame.message.as_deref())?;
                validate_optional_bounded("connId", frame.connection_id.as_deref())?;
                if let Some(arg) = frame.arg {
                    validate_required_bounded("arg.channel", &arg.channel)?;
                    validate_optional_bounded("arg.instId", arg.instrument.as_deref())?;
                }
                validate_optional_bounded("channel", frame.channel.as_deref())?;
                validate_optional_bounded("connCount", frame.connection_count.as_deref())?;
                Ok(DecodedPublicFrame::RejectedControl {
                    event: "error",
                    code: frame.code,
                    message: frame.message.unwrap_or_default(),
                })
            }
            "notice" => {
                let frame: WireNotice = decode_shape(value)?;
                validate_event(&frame.event, "notice")?;
                validate_optional_bounded("msg", frame.message.as_deref())?;
                validate_optional_bounded("connId", frame.connection_id.as_deref())?;
                Ok(DecodedPublicFrame::RejectedControl {
                    event: "notice",
                    code: frame.code,
                    message: frame.message.unwrap_or_default(),
                })
            }
            "channel-conn-count-error" => {
                let frame: WireServerError = decode_shape(value)?;
                validate_event(&frame.event, "channel-conn-count-error")?;
                validate_optional_bounded("msg", frame.message.as_deref())?;
                validate_optional_bounded("connId", frame.connection_id.as_deref())?;
                validate_optional_bounded("channel", frame.channel.as_deref())?;
                validate_optional_bounded("connCount", frame.connection_count.as_deref())?;
                if let Some(arg) = frame.arg {
                    validate_required_bounded("arg.channel", &arg.channel)?;
                    validate_optional_bounded("arg.instId", arg.instrument.as_deref())?;
                }
                Ok(DecodedPublicFrame::RejectedControl {
                    event: "channel-conn-count-error",
                    code: frame.code,
                    message: frame.message.unwrap_or_default(),
                })
            }
            "unsubscribe" => {
                let frame: WireAcknowledgement = decode_shape(value)?;
                validate_event(&frame.event, "unsubscribe")?;
                validate_optional_bounded("msg", frame.message.as_deref())?;
                validate_optional_bounded("connId", frame.connection_id.as_deref())?;
                Ok(DecodedPublicFrame::StateChangingControl {
                    event: "unsubscribe",
                })
            }
            _ => Err(PublicWireError::UnsupportedControlEvent {
                event: event.to_string(),
            }),
        };
    }
    let frame: WireDataFrame = decode_shape(value)?;
    if frame.data.len() > MAX_REFERENCE_VALUES_PER_FRAME {
        return Err(PublicWireError::TooManyValues);
    }
    Ok(DecodedPublicFrame::Data {
        arg: frame.arg,
        values: frame.data,
    })
}

fn decode_shape<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, PublicWireError> {
    serde_json::from_value(value).map_err(|error| PublicWireError::Malformed {
        reason: error.to_string(),
    })
}

fn validate_event(actual: &str, expected: &'static str) -> Result<(), PublicWireError> {
    if actual == expected {
        Ok(())
    } else {
        Err(PublicWireError::InvalidControlField { field: "event" })
    }
}

fn validate_required_bounded(field: &'static str, value: &str) -> Result<(), PublicWireError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(PublicWireError::InvalidControlField { field })
    } else {
        Ok(())
    }
}

fn validate_optional_bounded(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), PublicWireError> {
    if let Some(value) = value {
        validate_required_bounded(field, value)?;
    }
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum PublicWireError {
    #[error("OKX public frame exceeds its fixed byte bound")]
    FrameTooLarge,
    #[error("malformed OKX public frame: {reason}")]
    Malformed { reason: String },
    #[error("OKX public frame must be one JSON object")]
    NonObject,
    #[error("OKX public control event must be a string")]
    ControlEventNotString,
    #[error("unsupported OKX public control event {event:?}")]
    UnsupportedControlEvent { event: String },
    #[error("invalid OKX public control field {field}")]
    InvalidControlField { field: &'static str },
    #[error("OKX public data frame exceeds its value-count bound")]
    TooManyValues,
}
