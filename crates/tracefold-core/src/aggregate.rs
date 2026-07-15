use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{CanonicalEvent, Contract, Family, Measure, MeasureOp, QUERY_RESULT_SCHEMA_VERSION};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScalarValue {
    Null,
    String(String),
}

impl ScalarValue {
    pub fn from_option(value: Option<&str>) -> Self {
        value.map_or(Self::Null, |value| Self::String(value.to_owned()))
    }

    pub fn as_option(&self) -> Option<&str> {
        match self {
            Self::Null => None,
            Self::String(value) => Some(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySpec {
    pub family: String,
    pub start_ns: i64,
    pub end_ns: i64,
    #[serde(default)]
    pub filters: BTreeMap<String, Vec<ScalarValue>>,
    #[serde(default)]
    pub group_by: Vec<String>,
    #[serde(default)]
    pub measures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryResult {
    pub schema_version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_hash: Option<String>,
    pub contract_hash: String,
    pub exactness: String,
    pub family: String,
    pub start_ns: i64,
    pub end_ns: i64,
    pub filters: BTreeMap<String, Vec<ScalarValue>>,
    pub group_by: Vec<String>,
    pub measures: Vec<String>,
    pub rows: Vec<AggregateRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateRow {
    pub bucket_ns: i64,
    pub dimensions: BTreeMap<String, ScalarValue>,
    pub values: BTreeMap<String, AggregateValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AggregateValue {
    Count(u64),
    Integer(Option<i64>),
    Histogram(Vec<u64>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellKey {
    pub bucket_ns: i64,
    pub dimensions: Vec<ScalarValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellState {
    pub metrics: Vec<MetricState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricState {
    pub count: u64,
    pub present_count: u64,
    pub sum: i64,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub histogram: Vec<u64>,
}

#[derive(Debug, Error)]
pub enum AggregateError {
    #[error("query outside preserved contract: {0}")]
    Contract(String),
    #[error("integer overflow while aggregating `{0}`")]
    Overflow(String),
}

impl QuerySpec {
    pub fn validate<'a>(&self, contract: &'a Contract) -> Result<&'a Family, AggregateError> {
        let family = contract
            .family(&self.family)
            .ok_or_else(|| AggregateError::Contract(format!("unknown family `{}`", self.family)))?;
        let bucket = contract
            .bucket_ns()
            .map_err(|error| AggregateError::Contract(error.to_string()))?;
        if self.start_ns >= self.end_ns
            || self.start_ns.rem_euclid(bucket) != 0
            || self.end_ns.rem_euclid(bucket) != 0
        {
            return Err(AggregateError::Contract(
                "time range must be increasing and bucket-aligned".into(),
            ));
        }
        let dimensions: BTreeSet<&str> = family.dimensions.iter().map(String::as_str).collect();
        let mut groups = BTreeSet::new();
        for group in &self.group_by {
            if !dimensions.contains(group.as_str()) || !groups.insert(group) {
                return Err(AggregateError::Contract(format!(
                    "undeclared or duplicate grouping `{group}`"
                )));
            }
        }
        for (field, values) in &self.filters {
            if !dimensions.contains(field.as_str()) || values.is_empty() {
                return Err(AggregateError::Contract(format!(
                    "undeclared filter or empty IN set `{field}`"
                )));
            }
        }
        let available: BTreeSet<String> = family.measures.iter().map(measure_name).collect();
        let mut requested = BTreeSet::new();
        for measure in &self.measures {
            if !available.contains(measure) || !requested.insert(measure) {
                return Err(AggregateError::Contract(format!(
                    "undeclared or duplicate measure `{measure}`"
                )));
            }
        }
        Ok(family)
    }
}

impl CellState {
    pub fn new(family: &Family) -> Self {
        Self {
            metrics: family
                .measures
                .iter()
                .map(|measure| MetricState {
                    count: 0,
                    present_count: 0,
                    sum: 0,
                    min: None,
                    max: None,
                    histogram: if measure.operation == MeasureOp::Histogram {
                        vec![0; measure.bounds.len() + 1]
                    } else {
                        Vec::new()
                    },
                })
                .collect(),
        }
    }

    pub fn update(
        &mut self,
        family: &Family,
        event: &CanonicalEvent,
    ) -> Result<(), AggregateError> {
        for (measure, metric) in family.measures.iter().zip(&mut self.metrics) {
            metric.count = metric
                .count
                .checked_add(1)
                .ok_or_else(|| AggregateError::Overflow(measure_name(measure)))?;
            if measure.operation == MeasureOp::Count {
                continue;
            }
            let Some(value) = event.measure(&measure.field) else {
                continue;
            };
            metric.present_count = metric
                .present_count
                .checked_add(1)
                .ok_or_else(|| AggregateError::Overflow(measure_name(measure)))?;
            metric.sum = metric
                .sum
                .checked_add(value)
                .ok_or_else(|| AggregateError::Overflow(measure_name(measure)))?;
            metric.min = Some(metric.min.map_or(value, |old| old.min(value)));
            metric.max = Some(metric.max.map_or(value, |old| old.max(value)));
            if measure.operation == MeasureOp::Histogram {
                let bin = measure.bounds.partition_point(|bound| value >= *bound);
                metric.histogram[bin] = metric.histogram[bin]
                    .checked_add(1)
                    .ok_or_else(|| AggregateError::Overflow(measure_name(measure)))?;
            }
        }
        Ok(())
    }

    pub fn merge(&mut self, other: &Self, family: &Family) -> Result<(), AggregateError> {
        for ((left, right), measure) in self
            .metrics
            .iter_mut()
            .zip(&other.metrics)
            .zip(&family.measures)
        {
            left.count = left
                .count
                .checked_add(right.count)
                .ok_or_else(|| AggregateError::Overflow(measure_name(measure)))?;
            left.present_count = left
                .present_count
                .checked_add(right.present_count)
                .ok_or_else(|| AggregateError::Overflow(measure_name(measure)))?;
            left.sum = left
                .sum
                .checked_add(right.sum)
                .ok_or_else(|| AggregateError::Overflow(measure_name(measure)))?;
            left.min = match (left.min, right.min) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (value @ Some(_), None) | (None, value @ Some(_)) => value,
                (None, None) => None,
            };
            left.max = match (left.max, right.max) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (value @ Some(_), None) | (None, value @ Some(_)) => value,
                (None, None) => None,
            };
            for (left_bin, right_bin) in left.histogram.iter_mut().zip(&right.histogram) {
                *left_bin = left_bin
                    .checked_add(*right_bin)
                    .ok_or_else(|| AggregateError::Overflow(measure_name(measure)))?;
            }
        }
        Ok(())
    }

    pub fn selected_values(
        &self,
        family: &Family,
        requested: &BTreeSet<String>,
    ) -> BTreeMap<String, AggregateValue> {
        let all = requested.is_empty();
        family
            .measures
            .iter()
            .zip(&self.metrics)
            .filter_map(|(measure, metric)| {
                let name = measure_name(measure);
                if !all && !requested.contains(&name) {
                    return None;
                }
                let value = match measure.operation {
                    MeasureOp::Count => AggregateValue::Count(metric.count),
                    MeasureOp::CountPresent => AggregateValue::Count(metric.present_count),
                    MeasureOp::Sum => {
                        AggregateValue::Integer((metric.present_count > 0).then_some(metric.sum))
                    }
                    MeasureOp::Min => AggregateValue::Integer(metric.min),
                    MeasureOp::Max => AggregateValue::Integer(metric.max),
                    MeasureOp::Histogram => AggregateValue::Histogram(metric.histogram.clone()),
                };
                Some((name, value))
            })
            .collect()
    }
}

pub fn measure_name(measure: &Measure) -> String {
    if measure.operation == MeasureOp::Count {
        "count".into()
    } else {
        format!("{}:{}", measure.field, operation_name(measure.operation))
    }
}

fn operation_name(operation: MeasureOp) -> &'static str {
    match operation {
        MeasureOp::Count => "count",
        MeasureOp::CountPresent => "count_present",
        MeasureOp::Sum => "sum",
        MeasureOp::Min => "min",
        MeasureOp::Max => "max",
        MeasureOp::Histogram => "histogram",
    }
}

pub struct Oracle;

impl Oracle {
    pub fn query<'a>(
        contract: &Contract,
        query: &QuerySpec,
        events: impl IntoIterator<Item = &'a CanonicalEvent>,
    ) -> Result<QueryResult, AggregateError> {
        let family = query.validate(contract)?;
        let bucket_width = contract
            .bucket_ns()
            .map_err(|error| AggregateError::Contract(error.to_string()))?;
        let group_indices: Vec<usize> = query
            .group_by
            .iter()
            .map(|field| family.dimensions.iter().position(|d| d == field).unwrap())
            .collect();
        let mut cells: BTreeMap<CellKey, CellState> = BTreeMap::new();
        for event in events {
            if event.timestamp_ns < query.start_ns || event.timestamp_ns >= query.end_ns {
                continue;
            }
            let dimensions: Vec<ScalarValue> = family
                .dimensions
                .iter()
                .map(|field| ScalarValue::from_option(event.dimension(field)))
                .collect();
            if query.filters.iter().any(|(field, allowed)| {
                let index = family.dimensions.iter().position(|d| d == field).unwrap();
                !allowed.contains(&dimensions[index])
            }) {
                continue;
            }
            let key = CellKey {
                bucket_ns: event.timestamp_ns.div_euclid(bucket_width) * bucket_width,
                dimensions: group_indices
                    .iter()
                    .map(|index| dimensions[*index].clone())
                    .collect(),
            };
            cells
                .entry(key)
                .or_insert_with(|| CellState::new(family))
                .update(family, event)?;
        }
        rows_from_cells(contract, query, family, cells)
    }
}

/// A streaming raw-event oracle that retains aggregate state but never event bodies.
/// It is used by large-corpus benchmarks where holding every canonical event would
/// distort peak memory or exceed the host budget.
#[derive(Debug, Default)]
pub struct OracleIndex {
    families: BTreeMap<String, BTreeMap<CellKey, CellState>>,
}

impl OracleIndex {
    pub fn new(contract: &Contract) -> Self {
        Self {
            families: contract
                .families
                .iter()
                .map(|family| (family.name.clone(), BTreeMap::new()))
                .collect(),
        }
    }

    pub fn ingest(
        &mut self,
        contract: &Contract,
        event: &CanonicalEvent,
    ) -> Result<(), AggregateError> {
        let width = contract
            .bucket_ns()
            .map_err(|error| AggregateError::Contract(error.to_string()))?;
        for family in &contract.families {
            let key = CellKey {
                bucket_ns: event.timestamp_ns.div_euclid(width) * width,
                dimensions: family
                    .dimensions
                    .iter()
                    .map(|field| ScalarValue::from_option(event.dimension(field)))
                    .collect(),
            };
            self.families
                .get_mut(&family.name)
                .expect("contract family initialized")
                .entry(key)
                .or_insert_with(|| CellState::new(family))
                .update(family, event)?;
        }
        Ok(())
    }

    pub fn query(
        &self,
        contract: &Contract,
        query: &QuerySpec,
    ) -> Result<QueryResult, AggregateError> {
        let family = query.validate(contract)?;
        let source = self
            .families
            .get(&family.name)
            .ok_or_else(|| AggregateError::Contract("family index is unavailable".into()))?;
        let group_indices: Vec<_> = query
            .group_by
            .iter()
            .map(|field| {
                family
                    .dimensions
                    .iter()
                    .position(|dimension| dimension == field)
                    .expect("validated group dimension")
            })
            .collect();
        let filter_indices: Vec<_> = query
            .filters
            .iter()
            .map(|(field, allowed)| {
                (
                    family
                        .dimensions
                        .iter()
                        .position(|dimension| dimension == field)
                        .expect("validated filter dimension"),
                    allowed,
                )
            })
            .collect();
        let mut cells = BTreeMap::<CellKey, CellState>::new();
        for (key, state) in source {
            if key.bucket_ns < query.start_ns
                || key.bucket_ns >= query.end_ns
                || filter_indices
                    .iter()
                    .any(|(index, allowed)| !allowed.contains(&key.dimensions[*index]))
            {
                continue;
            }
            let grouped = CellKey {
                bucket_ns: key.bucket_ns,
                dimensions: group_indices
                    .iter()
                    .map(|index| key.dimensions[*index].clone())
                    .collect(),
            };
            if let Some(existing) = cells.get_mut(&grouped) {
                existing.merge(state, family)?;
            } else {
                cells.insert(grouped, state.clone());
            }
        }
        rows_from_cells(contract, query, family, cells)
    }

    pub fn dimension_values(
        &self,
        contract: &Contract,
        family_name: &str,
        dimension: &str,
        limit: usize,
    ) -> Vec<ScalarValue> {
        let Some(family) = contract.family(family_name) else {
            return Vec::new();
        };
        let Some(index) = family
            .dimensions
            .iter()
            .position(|candidate| candidate == dimension)
        else {
            return Vec::new();
        };
        self.families[family_name]
            .keys()
            .map(|key| key.dimensions[index].clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(limit)
            .collect()
    }
}

pub fn rows_from_cells(
    contract: &Contract,
    query: &QuerySpec,
    family: &Family,
    cells: BTreeMap<CellKey, CellState>,
) -> Result<QueryResult, AggregateError> {
    let requested: BTreeSet<String> = query.measures.iter().cloned().collect();
    let rows = cells
        .into_iter()
        .map(|(key, cell)| AggregateRow {
            bucket_ns: key.bucket_ns,
            dimensions: query.group_by.iter().cloned().zip(key.dimensions).collect(),
            values: cell.selected_values(family, &requested),
        })
        .collect();
    Ok(QueryResult {
        schema_version: QUERY_RESULT_SCHEMA_VERSION,
        archive_hash: None,
        contract_hash: contract.hash().to_hex().to_string(),
        exactness: "exact".into(),
        family: query.family.clone(),
        start_ns: query.start_ns,
        end_ns: query.end_ns,
        filters: query.filters.clone(),
        group_by: query.group_by.clone(),
        measures: if query.measures.is_empty() {
            family.measures.iter().map(measure_name).collect()
        } else {
            query.measures.clone()
        },
        rows,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;

    use super::*;
    use crate::{Severity, Status};

    fn event(timestamp_ns: i64, service: &str, duration: Option<i64>) -> CanonicalEvent {
        CanonicalEvent {
            schema_version: 1,
            timestamp_ns,
            event_id: format!("{service}-{timestamp_ns}"),
            trace_id: None,
            span_id: None,
            parent_span_id: None,
            service: service.into(),
            operation: Some("op".into()),
            event_type: "done".into(),
            severity: Severity::Info,
            status: Status::Ok,
            error_code: None,
            model: None,
            duration_ns: duration,
            bytes_in: None,
            bytes_out: None,
            tokens_in: None,
            tokens_out: None,
            attributes: BTreeMap::new(),
            body: Value::Null,
        }
    }

    #[test]
    fn oracle_groups_and_ignores_null_measures() {
        let contract =
            Contract::parse(include_str!("../../../contracts/telemetry-v1.toml").to_owned())
                .unwrap();
        let events = [event(0, "a", Some(10)), event(1, "a", None)];
        let query = QuerySpec {
            family: "latency".into(),
            start_ns: 0,
            end_ns: 60_000_000_000,
            filters: BTreeMap::new(),
            group_by: vec!["service".into()],
            measures: vec!["duration_ns:count_present".into(), "duration_ns:sum".into()],
        };
        let result = Oracle::query(&contract, &query, &events).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].values["duration_ns:count_present"],
            AggregateValue::Count(1)
        );
        let mut index = OracleIndex::new(&contract);
        for event in &events {
            index.ingest(&contract, event).unwrap();
        }
        assert_eq!(index.query(&contract, &query).unwrap(), result);
    }
}
