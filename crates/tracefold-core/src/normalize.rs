use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde_json::json;

use crate::{CanonicalEvent, Severity, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapter {
    LoghubZookeeper,
    LoghubBgl,
}

impl std::str::FromStr for Adapter {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "loghub-zookeeper" => Ok(Self::LoghubZookeeper),
            "loghub-bgl" => Ok(Self::LoghubBgl),
            _ => Err(format!("unknown adapter `{value}`")),
        }
    }
}

pub fn normalize_line(adapter: Adapter, line: &str, index: u64) -> (CanonicalEvent, bool) {
    match adapter {
        Adapter::LoghubZookeeper => normalize_zookeeper(line, index),
        Adapter::LoghubBgl => normalize_bgl(line, index),
    }
}

fn normalize_zookeeper(line: &str, index: u64) -> (CanonicalEvent, bool) {
    let parts: Vec<&str> = line.splitn(5, " - ").collect();
    let parsed = parts.len() >= 2;
    let timestamp = parts
        .first()
        .and_then(|value| parse_timestamp(value, "%Y-%m-%d %H:%M:%S,%3f"))
        .unwrap_or(index as i64);
    let context = parts.get(1).copied().unwrap_or_default();
    let level = context
        .split_whitespace()
        .find(|value| matches!(*value, "DEBUG" | "INFO" | "WARN" | "ERROR" | "FATAL"));
    let severity = severity(level);
    let operation = context
        .split_once(':')
        .map(|(_, logger)| logger)
        .and_then(|logger| logger.split('@').next())
        .unwrap_or("zookeeper");
    (
        public_event(
            "zookeeper",
            operation,
            if parsed { "log.record" } else { "unparsed" },
            timestamp,
            index,
            severity,
            line,
            BTreeMap::new(),
        ),
        !parsed,
    )
}

fn normalize_bgl(line: &str, index: u64) -> (CanonicalEvent, bool) {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let parsed = fields.len() >= 9;
    let alert = fields.first().is_some_and(|value| *value != "-");
    let timestamp = fields
        .get(1)
        .and_then(|value| value.parse::<i64>().ok())
        .map(|seconds| seconds.saturating_mul(1_000_000_000))
        .unwrap_or(index as i64);
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "node".into(),
        fields.get(3).map(|value| (*value).to_owned()),
    );
    attributes.insert(
        "facility".into(),
        fields.get(6).map(|value| (*value).to_owned()),
    );
    (
        public_event(
            "bgl",
            fields.get(7).copied().unwrap_or("bgl"),
            if parsed { "log.record" } else { "unparsed" },
            timestamp,
            index,
            if alert {
                Severity::Error
            } else {
                Severity::Info
            },
            line,
            attributes,
        ),
        !parsed,
    )
}

#[allow(clippy::too_many_arguments)]
fn public_event(
    service: &str,
    operation: &str,
    event_type: &str,
    timestamp_ns: i64,
    index: u64,
    severity: Severity,
    raw: &str,
    attributes: BTreeMap<String, Option<String>>,
) -> CanonicalEvent {
    let error = matches!(severity, Severity::Error | Severity::Fatal);
    CanonicalEvent {
        schema_version: 1,
        timestamp_ns,
        event_id: format!("{service}-{index:012}"),
        trace_id: None,
        span_id: None,
        parent_span_id: None,
        service: service.into(),
        operation: Some(operation.into()),
        event_type: event_type.into(),
        severity,
        status: if error { Status::Error } else { Status::Ok },
        error_code: error.then(|| "LOG_ALERT".into()),
        model: None,
        duration_ns: None,
        bytes_in: None,
        bytes_out: None,
        tokens_in: None,
        tokens_out: None,
        attributes,
        body: json!({"raw_line": raw}),
    }
}

fn severity(value: Option<&str>) -> Severity {
    match value {
        Some("DEBUG") => Severity::Debug,
        Some("INFO") => Severity::Info,
        Some("WARN") => Severity::Warn,
        Some("ERROR") => Severity::Error,
        Some("FATAL") => Severity::Fatal,
        _ => Severity::Unknown,
    }
}

fn parse_timestamp(value: &str, format: &str) -> Option<i64> {
    NaiveDateTime::parse_from_str(value, format)
        .ok()
        .map(|value| {
            Utc.from_utc_datetime(&value)
                .timestamp_nanos_opt()
                .unwrap_or_default()
        })
        .or_else(|| {
            DateTime::parse_from_rfc3339(value)
                .ok()?
                .timestamp_nanos_opt()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_lines_are_retained_as_unparsed() {
        let (event, warning) = normalize_line(Adapter::LoghubBgl, "short", 1);
        assert!(warning);
        assert_eq!(event.event_type, "unparsed");
        assert_eq!(event.body["raw_line"], "short");
    }

    #[test]
    fn zookeeper_mapping_parses_timestamp_level_and_logger() {
        let line =
            "2015-07-29 17:41:41,536 - INFO  [main:QuorumPeerConfig@101] - Reading configuration";
        let (event, warning) = normalize_line(Adapter::LoghubZookeeper, line, 0);
        assert!(!warning);
        assert_eq!(event.operation.as_deref(), Some("QuorumPeerConfig"));
        assert_eq!(event.severity, Severity::Info);
        assert!(event.timestamp_ns > 1_400_000_000_000_000_000);
    }

    #[test]
    fn bgl_mapping_uses_epoch_node_type_and_component() {
        let line = "- 1117838570 2005.06.03 R02-M1-N0-C:J12-U11 2005-06-03-15.42.50.363779 R02-M1-N0-C:J12-U11 RAS KERNEL INFO cache corrected";
        let (event, warning) = normalize_line(Adapter::LoghubBgl, line, 0);
        assert!(!warning);
        assert_eq!(event.timestamp_ns, 1_117_838_570_000_000_000);
        assert_eq!(event.operation.as_deref(), Some("KERNEL"));
        assert_eq!(event.attributes["facility"].as_deref(), Some("RAS"));
    }
}
