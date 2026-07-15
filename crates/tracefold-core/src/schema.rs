use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::CANONICAL_SCHEMA_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    Unknown,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Ok,
    Error,
    Unset,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Error => "ERROR",
            Self::Unset => "UNSET",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalEvent {
    pub schema_version: u16,
    pub timestamp_ns: i64,
    pub event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub service: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    pub event_type: String,
    pub severity: Severity,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ns: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_in: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_out: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<i64>,
    #[serde(default)]
    pub attributes: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub body: Value,
}

#[derive(Debug, Error)]
pub enum EventError {
    #[error("invalid canonical JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported canonical schema version {0}")]
    SchemaVersion(u16),
    #[error("required field `{0}` must not be empty")]
    Empty(&'static str),
}

impl CanonicalEvent {
    pub fn parse_line(line: &str) -> Result<Self, EventError> {
        let event: Self = serde_json::from_str(line)?;
        event.validate()?;
        Ok(event)
    }

    pub fn validate(&self) -> Result<(), EventError> {
        if self.schema_version != CANONICAL_SCHEMA_VERSION {
            return Err(EventError::SchemaVersion(self.schema_version));
        }
        for (name, value) in [
            ("event_id", self.event_id.as_str()),
            ("service", self.service.as_str()),
            ("event_type", self.event_type.as_str()),
        ] {
            if value.is_empty() {
                return Err(EventError::Empty(name));
            }
        }
        Ok(())
    }

    pub fn canonical_line(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn dimension(&self, field: &str) -> Option<&str> {
        match field {
            "service" => Some(self.service.as_str()),
            "operation" => self.operation.as_deref(),
            "event_type" => Some(self.event_type.as_str()),
            "severity" => Some(self.severity.as_str()),
            "status" => Some(self.status.as_str()),
            "error_code" => self.error_code.as_deref(),
            "model" => self.model.as_deref(),
            "trace_id" => self.trace_id.as_deref(),
            "span_id" => self.span_id.as_deref(),
            "parent_span_id" => self.parent_span_id.as_deref(),
            field if field.starts_with("attributes.") => self
                .attributes
                .get(&field["attributes.".len()..])
                .and_then(Option::as_deref),
            _ => None,
        }
    }

    pub fn measure(&self, field: &str) -> Option<i64> {
        match field {
            "duration_ns" => self.duration_ns,
            "bytes_in" => self.bytes_in,
            "bytes_out" => self.bytes_out,
            "tokens_in" => self.tokens_in,
            "tokens_out" => self.tokens_out,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_fields() {
        let source = r#"{"schema_version":1,"timestamp_ns":0,"event_id":"e","service":"s","event_type":"x","severity":"INFO","status":"OK","surprise":1}"#;
        assert!(CanonicalEvent::parse_line(source).is_err());
    }

    #[test]
    fn canonical_round_trip_is_stable() {
        let event = CanonicalEvent::parse_line(
            r#"{"schema_version":1,"timestamp_ns":1,"event_id":"e","service":"s","event_type":"x","severity":"INFO","status":"OK","attributes":{"z":"a","a":null}}"#,
        )
        .unwrap();
        let line = event.canonical_line().unwrap();
        assert_eq!(CanonicalEvent::parse_line(&line).unwrap(), event);
        assert!(line.find("\"a\"").unwrap() < line.find("\"z\"").unwrap());
    }
}
