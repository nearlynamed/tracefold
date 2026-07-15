use std::{collections::BTreeSet, fs, path::Path};

use blake3::Hash;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::CONTRACT_SCHEMA_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub version: u16,
    pub name: String,
    pub time_bucket: String,
    pub retention: Retention,
    pub families: Vec<Family>,
    #[serde(skip)]
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    pub recent: String,
    pub error_severities: Vec<String>,
    pub error_statuses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecentRetention {
    Duration(i64),
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Family {
    pub name: String,
    pub dimensions: Vec<String>,
    pub measures: Vec<Measure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measure {
    pub field: String,
    #[serde(rename = "op")]
    pub operation: MeasureOp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bounds: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureOp {
    Count,
    CountPresent,
    Sum,
    Min,
    Max,
    Histogram,
}

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("cannot read contract: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid TOML contract: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("unsupported contract version {0}")]
    Version(u16),
    #[error("invalid duration `{0}`")]
    Duration(String),
    #[error("invalid contract: {0}")]
    Validation(String),
}

const DAY_NS: i64 = 86_400_000_000_000;

impl Contract {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ContractError> {
        let source = fs::read_to_string(path)?;
        Self::parse(source)
    }

    pub fn parse(source: String) -> Result<Self, ContractError> {
        let mut contract: Self = toml::from_str(&source)?;
        contract.source = source;
        contract.validate()?;
        Ok(contract)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.version != CONTRACT_SCHEMA_VERSION {
            return Err(ContractError::Version(self.version));
        }
        if self.name.is_empty() || self.families.is_empty() {
            return Err(ContractError::Validation(
                "name and at least one family are required".into(),
            ));
        }
        let bucket = self.bucket_ns()?;
        if !(1_000_000_000..=DAY_NS).contains(&bucket) || DAY_NS % bucket != 0 {
            return Err(ContractError::Validation(
                "time_bucket must be 1s..1d and divide one day evenly".into(),
            ));
        }
        self.recent_retention()?;
        let valid_dimensions = [
            "service",
            "operation",
            "event_type",
            "severity",
            "status",
            "error_code",
            "model",
            "trace_id",
            "span_id",
            "parent_span_id",
        ];
        let valid_measures = [
            "duration_ns",
            "bytes_in",
            "bytes_out",
            "tokens_in",
            "tokens_out",
        ];
        let mut names = BTreeSet::new();
        for family in &self.families {
            if family.name.is_empty() || !names.insert(&family.name) {
                return Err(ContractError::Validation(format!(
                    "duplicate or empty family name `{}`",
                    family.name
                )));
            }
            if !(1..=6).contains(&family.dimensions.len()) {
                return Err(ContractError::Validation(format!(
                    "family `{}` must declare 1..6 dimensions",
                    family.name
                )));
            }
            let mut dimensions = BTreeSet::new();
            for field in &family.dimensions {
                if (!valid_dimensions.contains(&field.as_str())
                    && !field.starts_with("attributes."))
                    || !dimensions.insert(field)
                {
                    return Err(ContractError::Validation(format!(
                        "invalid or duplicate dimension `{field}`"
                    )));
                }
            }
            let mut measures = BTreeSet::new();
            for measure in &family.measures {
                if measure.operation == MeasureOp::Count {
                    if measure.field != "*" {
                        return Err(ContractError::Validation("count requires field `*`".into()));
                    }
                } else if !valid_measures.contains(&measure.field.as_str()) {
                    return Err(ContractError::Validation(format!(
                        "invalid measure field `{}`",
                        measure.field
                    )));
                }
                if !measures.insert((measure.field.as_str(), measure.operation)) {
                    return Err(ContractError::Validation(format!(
                        "duplicate measure {}:{:?}",
                        measure.field, measure.operation
                    )));
                }
                if measure.operation == MeasureOp::Histogram {
                    if measure.bounds.windows(2).any(|w| w[0] >= w[1]) {
                        return Err(ContractError::Validation(
                            "histogram bounds must be sorted and unique".into(),
                        ));
                    }
                } else if !measure.bounds.is_empty() {
                    return Err(ContractError::Validation(
                        "bounds are valid only for histograms".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn bucket_ns(&self) -> Result<i64, ContractError> {
        parse_duration_ns(&self.time_bucket)
    }

    pub fn recent_retention(&self) -> Result<RecentRetention, ContractError> {
        if self.retention.recent == "all" {
            Ok(RecentRetention::All)
        } else if self.retention.recent == "0" {
            Ok(RecentRetention::Duration(0))
        } else {
            Ok(RecentRetention::Duration(parse_duration_ns(
                &self.retention.recent,
            )?))
        }
    }

    pub fn hash(&self) -> Hash {
        blake3::hash(self.source.as_bytes())
    }

    pub fn family(&self, name: &str) -> Option<&Family> {
        self.families.iter().find(|family| family.name == name)
    }
}

pub fn parse_duration_ns(value: &str) -> Result<i64, ContractError> {
    let split = value
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| ContractError::Duration(value.into()))?;
    let (number, unit) = value.split_at(split);
    if number.is_empty() || unit.is_empty() || number.starts_with('0') && number.len() > 1 {
        return Err(ContractError::Duration(value.into()));
    }
    let number: i64 = number
        .parse()
        .map_err(|_| ContractError::Duration(value.into()))?;
    let multiplier: i64 = match unit {
        "ns" => 1,
        "us" => 1_000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        "d" => DAY_NS,
        _ => return Err(ContractError::Duration(value.into())),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| ContractError::Duration(value.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_integer_and_checked() {
        assert_eq!(parse_duration_ns("1m").unwrap(), 60_000_000_000);
        assert!(parse_duration_ns("1.5m").is_err());
        assert!(parse_duration_ns("60").is_err());
    }

    #[test]
    fn default_contract_validates() {
        let source = include_str!("../../../contracts/telemetry-v1.toml").to_owned();
        assert!(Contract::parse(source).is_ok());
    }
}
