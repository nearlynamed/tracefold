use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracefold_core::{
    CanonicalEvent, CellKey, CellState, Contract, QueryResult, QuerySpec, ScalarValue,
    aggregate::rows_from_cells,
};

use crate::{
    ARCHIVE_FORMAT_VERSION,
    codec::{CodecError, ViewReader},
    raw_codec::RawEventReader,
    writer::{ArchiveMeta, ChecksumEntry, ComponentSize, ViewMeta, is_error},
};

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("archive I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("archive contract error: {0}")]
    Contract(#[from] tracefold_core::contract::ContractError),
    #[error("view codec error: {0}")]
    Codec(#[from] CodecError),
    #[error("corrupt or unsupported archive: {0}")]
    Corrupt(String),
    #[error("query outside preserved contract: {0}")]
    Query(String),
    #[error("semantic verification mismatch: {0}")]
    Mismatch(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedClass {
    All,
    Recent,
    Errors,
}

impl std::str::FromStr for RetainedClass {
    type Err = ArchiveError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "all" => Ok(Self::All),
            "recent" => Ok(Self::Recent),
            "errors" => Ok(Self::Errors),
            _ => Err(ArchiveError::Query(format!(
                "unknown retention class `{value}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResult {
    pub schema_version: u16,
    pub archive_hash: String,
    pub contract_hash: String,
    pub record_count: u64,
    pub error_count: u64,
    pub recent_count: u64,
    pub min_timestamp_ns: i64,
    pub max_timestamp_ns: i64,
    pub hot_cutoff_ns: i64,
    pub bucket_width_ns: i64,
    pub requested_layout: crate::Layout,
    pub selected_layout: crate::Layout,
    pub candidate_archive_bytes: BTreeMap<String, u64>,
    pub components: BTreeMap<String, ComponentSize>,
    pub views: Vec<PublicView>,
    pub dictionary_cardinalities: BTreeMap<String, u64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicView {
    pub id: String,
    pub families: Vec<String>,
    pub dimensions: Vec<String>,
    pub row_count: u64,
    pub block_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub schema_version: u16,
    pub archive_hash: String,
    pub files_checked: u64,
    pub views_checked: u64,
    pub rows_checked: u64,
    pub valid: bool,
}

pub struct Archive {
    root: PathBuf,
    meta: ArchiveMeta,
    contract: Contract,
    dictionaries: BTreeMap<String, Vec<Option<String>>>,
}

impl Archive {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let root = path.as_ref().to_owned();
        if !root.is_dir() {
            return Err(ArchiveError::Corrupt(
                "archive path is not a directory".into(),
            ));
        }
        let meta: ArchiveMeta = read_json(&root.join("meta.json"))?;
        if meta.archive_version != ARCHIVE_FORMAT_VERSION {
            return Err(ArchiveError::Corrupt(format!(
                "unsupported archive version {}",
                meta.archive_version
            )));
        }
        let contract = Contract::load(root.join("contract.toml"))?;
        if contract.hash().to_hex().as_str() != meta.contract_hash {
            return Err(ArchiveError::Corrupt(
                "embedded contract hash mismatch".into(),
            ));
        }
        let mut dictionaries = BTreeMap::new();
        for view in &meta.views {
            for dimension in &view.dimensions {
                if dictionaries.contains_key(dimension) {
                    continue;
                }
                let path = root
                    .join("dictionaries")
                    .join(format!("{}.json.zst", safe_name(dimension)));
                let decoder = zstd::stream::read::Decoder::new(File::open(path)?)?;
                let values: Vec<Option<String>> = serde_json::from_reader(decoder)?;
                if values.first() != Some(&None) {
                    return Err(ArchiveError::Corrupt(format!(
                        "dictionary `{dimension}` does not reserve ID zero for null"
                    )));
                }
                if values
                    .iter()
                    .skip(1)
                    .collect::<Vec<_>>()
                    .windows(2)
                    .any(|window| window[0] >= window[1])
                {
                    return Err(ArchiveError::Corrupt(format!(
                        "dictionary `{dimension}` is not strictly sorted"
                    )));
                }
                dictionaries.insert(dimension.clone(), values);
            }
        }
        Ok(Self {
            root,
            meta,
            contract,
            dictionaries,
        })
    }

    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    pub fn inspect(&self) -> InspectResult {
        let mut components = self.meta.components.clone();
        let known = components
            .values()
            .map(|component| component.compressed_bytes)
            .sum::<u64>();
        if let Ok(total) = directory_bytes(&self.root) {
            let bytes = total.saturating_sub(known);
            components.insert(
                "metadata".into(),
                ComponentSize {
                    logical_bytes: bytes,
                    compressed_bytes: bytes,
                },
            );
        }
        InspectResult {
            schema_version: 1,
            archive_hash: self.meta.archive_hash.clone(),
            contract_hash: self.meta.contract_hash.clone(),
            record_count: self.meta.record_count,
            error_count: self.meta.error_count,
            recent_count: self.meta.recent_count,
            min_timestamp_ns: self.meta.min_timestamp_ns,
            max_timestamp_ns: self.meta.max_timestamp_ns,
            hot_cutoff_ns: self.meta.hot_cutoff_ns,
            bucket_width_ns: self.meta.bucket_width_ns,
            requested_layout: self.meta.requested_layout,
            selected_layout: self.meta.layout,
            candidate_archive_bytes: self.meta.candidate_archive_bytes.clone(),
            components,
            views: self
                .meta
                .views
                .iter()
                .map(|view| PublicView {
                    id: view.id.clone(),
                    families: view.families.clone(),
                    dimensions: view.dimensions.clone(),
                    row_count: view.row_count,
                    block_count: view.block_count,
                })
                .collect(),
            dictionary_cardinalities: self.meta.dictionary_cardinalities.clone(),
            warnings: self.meta.warnings.clone(),
        }
    }

    pub fn query(&self, query: &QuerySpec) -> Result<QueryResult, ArchiveError> {
        let family = query
            .validate(&self.contract)
            .map_err(|error| ArchiveError::Query(error.to_string()))?
            .clone();
        let view = self
            .meta
            .views
            .iter()
            .find(|view| view.families.contains(&family.name))
            .ok_or_else(|| {
                ArchiveError::Query(format!("family `{}` is unavailable", family.name))
            })?;
        let indices = view
            .family_measure_indices
            .get(&family.name)
            .ok_or_else(|| ArchiveError::Corrupt("missing family measure mapping".into()))?;
        if indices.len() != family.measures.len() {
            return Err(ArchiveError::Corrupt(
                "family measure mapping length mismatch".into(),
            ));
        }
        let mut reader = ViewReader::open(&self.root.join(&view.path))?;
        if reader.schema.dimensions != view.dimensions || reader.schema.measures != view.measures {
            return Err(ArchiveError::Corrupt(
                "view metadata/schema mismatch".into(),
            ));
        }
        let group_indices: Vec<usize> = query
            .group_by
            .iter()
            .map(|field| {
                view.dimensions
                    .iter()
                    .position(|dimension| dimension == field)
                    .unwrap()
            })
            .collect();
        let filter_indices: Vec<(usize, &Vec<ScalarValue>)> = query
            .filters
            .iter()
            .map(|(field, allowed)| {
                (
                    view.dimensions
                        .iter()
                        .position(|dimension| dimension == field)
                        .unwrap(),
                    allowed,
                )
            })
            .collect();
        let mut cells = BTreeMap::<CellKey, CellState>::new();
        for row in reader.rows(Some(query.start_ns), Some(query.end_ns))? {
            if row.key.bucket_ns < query.start_ns || row.key.bucket_ns >= query.end_ns {
                continue;
            }
            let dimensions = self.decode_dimensions(view, &row.key.dimensions)?;
            if filter_indices
                .iter()
                .any(|(index, allowed)| !allowed.contains(&dimensions[*index]))
            {
                continue;
            }
            let state = CellState {
                metrics: indices
                    .iter()
                    .map(|index| {
                        row.state.metrics.get(*index).cloned().ok_or_else(|| {
                            ArchiveError::Corrupt("measure index out of bounds".into())
                        })
                    })
                    .collect::<Result<_, _>>()?,
            };
            let key = CellKey {
                bucket_ns: row.key.bucket_ns,
                dimensions: group_indices
                    .iter()
                    .map(|index| dimensions[*index].clone())
                    .collect(),
            };
            if let Some(existing) = cells.get_mut(&key) {
                existing
                    .merge(&state, &family)
                    .map_err(|error| ArchiveError::Corrupt(error.to_string()))?;
            } else {
                cells.insert(key, state);
            }
        }
        let mut result = rows_from_cells(&self.contract, query, &family, cells)
            .map_err(|error| ArchiveError::Query(error.to_string()))?;
        result.archive_hash = Some(self.meta.archive_hash.clone());
        Ok(result)
    }

    fn decode_dimensions(
        &self,
        view: &ViewMeta,
        ids: &[ScalarValue],
    ) -> Result<Vec<ScalarValue>, ArchiveError> {
        if ids.len() != view.dimensions.len() {
            return Err(ArchiveError::Corrupt("dimension count mismatch".into()));
        }
        view.dimensions
            .iter()
            .zip(ids)
            .map(|(field, id)| match id {
                ScalarValue::Null => Ok(ScalarValue::Null),
                ScalarValue::String(id) => {
                    let id: usize = id
                        .parse()
                        .map_err(|_| ArchiveError::Corrupt("non-numeric dictionary ID".into()))?;
                    self.dictionaries[field]
                        .get(id)
                        .ok_or_else(|| ArchiveError::Corrupt("dictionary ID out of bounds".into()))?
                        .as_ref()
                        .map_or_else(
                            || Ok(ScalarValue::Null),
                            |value| Ok(ScalarValue::String(value.clone())),
                        )
                }
            })
            .collect()
    }

    pub fn retained_events(
        &self,
        class: RetainedClass,
    ) -> Result<Vec<CanonicalEvent>, ArchiveError> {
        let mut events = Vec::new();
        if matches!(class, RetainedClass::All | RetainedClass::Recent) {
            events.extend(read_raw_events(&self.root.join("raw/recent.tfr"))?);
        }
        if matches!(class, RetainedClass::All | RetainedClass::Errors) {
            events.extend(read_raw_events(&self.root.join("raw/errors.tfr"))?);
        }
        events.sort_by(|left, right| {
            (left.timestamp_ns, left.event_id.as_str())
                .cmp(&(right.timestamp_ns, right.event_id.as_str()))
        });
        Ok(events)
    }

    pub fn verify(&self) -> Result<VerificationReport, ArchiveError> {
        let checksums: BTreeMap<String, ChecksumEntry> =
            read_json(&self.root.join("checksums.json"))?;
        let mut files_checked = 0_u64;
        for (relative, expected) in &checksums {
            let relative_path = Path::new(relative);
            if relative_path.is_absolute()
                || relative_path
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(ArchiveError::Corrupt("unsafe checksum path".into()));
            }
            let path = self.root.join(relative_path);
            let metadata = fs::metadata(&path)?;
            if !metadata.is_file() || metadata.len() != expected.bytes {
                return Err(ArchiveError::Corrupt(format!(
                    "length mismatch for `{relative}`"
                )));
            }
            let mut file = File::open(path)?;
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher)?;
            if hasher.finalize().to_hex().as_str() != expected.blake3 {
                return Err(ArchiveError::Corrupt(format!(
                    "checksum mismatch for `{relative}`"
                )));
            }
            files_checked += 1;
        }
        let mut rows_checked = 0_u64;
        for view in &self.meta.views {
            let mut reader = ViewReader::open(&self.root.join(&view.path))?;
            if reader.bucket_width != self.meta.bucket_width_ns {
                return Err(ArchiveError::Corrupt(format!(
                    "bucket width mismatch for `{}`",
                    view.id
                )));
            }
            let rows = reader.rows(None, None)?;
            if rows.len() as u64 != view.row_count {
                return Err(ArchiveError::Corrupt(format!(
                    "row count mismatch for `{}`",
                    view.id
                )));
            }
            let mut previous: Option<&CellKey> = None;
            for row in &rows {
                if previous.is_some_and(|previous| previous >= &row.key) {
                    return Err(ArchiveError::Corrupt(format!(
                        "view `{}` is not strictly sorted",
                        view.id
                    )));
                }
                self.decode_dimensions(view, &row.key.dimensions)?;
                verify_state(&row.state, &view.measures)?;
                previous = Some(&row.key);
            }
            rows_checked += rows.len() as u64;
        }
        let recent = read_raw_events(&self.root.join("raw/recent.tfr"))?;
        let errors = read_raw_events(&self.root.join("raw/errors.tfr"))?;
        if errors.len() as u64 != self.meta.error_count
            || recent.len() as u64 != self.meta.recent_count
            || recent.iter().any(|event| {
                event.timestamp_ns < self.meta.hot_cutoff_ns || is_error(&self.contract, event)
            })
            || errors.iter().any(|event| !is_error(&self.contract, event))
        {
            return Err(ArchiveError::Corrupt(
                "retained-event classification mismatch".into(),
            ));
        }
        let recent_ids: std::collections::BTreeSet<_> =
            recent.iter().map(|event| &event.event_id).collect();
        if errors
            .iter()
            .any(|event| recent_ids.contains(&event.event_id))
        {
            return Err(ArchiveError::Corrupt(
                "recent and error retention tiers overlap".into(),
            ));
        }
        Ok(VerificationReport {
            schema_version: 1,
            archive_hash: self.meta.archive_hash.clone(),
            files_checked,
            views_checked: self.meta.views.len() as u64,
            rows_checked,
            valid: true,
        })
    }

    pub fn verify_queries(
        &self,
        source: &Path,
        queries: &[QuerySpec],
    ) -> Result<VerificationReport, ArchiveError> {
        let report = self.verify()?;
        let events = read_plain_events(source)?;
        for (index, query) in queries.iter().enumerate() {
            let expected = tracefold_core::Oracle::query(&self.contract, query, &events)
                .map_err(|error| ArchiveError::Mismatch(error.to_string()))?;
            let actual = self.query(query)?;
            if expected.rows != actual.rows
                || expected.family != actual.family
                || expected.start_ns != actual.start_ns
                || expected.end_ns != actual.end_ns
                || expected.filters != actual.filters
                || expected.group_by != actual.group_by
                || expected.measures != actual.measures
            {
                return Err(ArchiveError::Mismatch(format!(
                    "query {index} differs from raw oracle"
                )));
            }
        }
        Ok(report)
    }
}

fn verify_state(
    state: &CellState,
    measures: &[tracefold_core::Measure],
) -> Result<(), ArchiveError> {
    if state.metrics.len() != measures.len() {
        return Err(ArchiveError::Corrupt("metric count mismatch".into()));
    }
    for (metric, measure) in state.metrics.iter().zip(measures) {
        if metric.present_count > metric.count
            || metric
                .min
                .zip(metric.max)
                .is_some_and(|(min, max)| min > max)
            || (measure.operation == tracefold_core::MeasureOp::Histogram
                && metric
                    .histogram
                    .iter()
                    .try_fold(0_u64, |sum, value| sum.checked_add(*value))
                    != Some(metric.present_count))
        {
            return Err(ArchiveError::Corrupt("aggregate invariant failed".into()));
        }
    }
    Ok(())
}

fn read_raw_events(path: &Path) -> Result<Vec<CanonicalEvent>, ArchiveError> {
    RawEventReader::open(path)
        .and_then(|mut reader| reader.events())
        .map_err(|error| ArchiveError::Corrupt(error.to_string()))
}

fn read_plain_events(path: &Path) -> Result<Vec<CanonicalEvent>, ArchiveError> {
    read_event_reader(BufReader::new(File::open(path)?))
}

fn read_event_reader(reader: impl BufRead) -> Result<Vec<CanonicalEvent>, ArchiveError> {
    reader
        .lines()
        .map(|line| {
            CanonicalEvent::parse_line(&line?)
                .map_err(|error| ArchiveError::Corrupt(error.to_string()))
        })
        .collect()
}

fn safe_name(field: &str) -> String {
    field
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn directory_bytes(root: &Path) -> std::io::Result<u64> {
    fn visit(path: &Path, total: &mut u64) -> std::io::Result<()> {
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(&path, total)?;
            } else if path.is_file() {
                *total = total.saturating_add(fs::metadata(path)?.len());
            }
        }
        Ok(())
    }
    let mut total = 0;
    visit(root, &mut total)?;
    Ok(total)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ArchiveError> {
    Ok(serde_json::from_reader(BufReader::new(File::open(path)?))?)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tracefold_core::generator::{GeneratorConfig, Scenario, generate};

    use super::*;
    use crate::{EncodeOptions, Layout, encode};

    #[test]
    fn encode_query_verify_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("events.jsonl");
        let mut writer = File::create(&input).unwrap();
        generate(
            &GeneratorConfig {
                scenario: Scenario::Standard,
                events: Some(500),
                max_output_bytes: None,
                seed: 7,
            },
            &mut writer,
        )
        .unwrap();
        writer.flush().unwrap();
        let contract =
            Contract::parse(include_str!("../../../contracts/telemetry-v1.toml").to_owned())
                .unwrap();
        let output = dir.path().join("test.tfold");
        encode(
            &input,
            &contract,
            &output,
            &EncodeOptions {
                layout: Layout::Separate,
                aggregation_budget_bytes: 1,
                ..EncodeOptions::default()
            },
        )
        .unwrap();
        let archive = Archive::open(&output).unwrap();
        assert!(archive.verify().unwrap().valid);
        let query = QuerySpec {
            family: "event-volume".into(),
            start_ns: 1_784_064_000_000_000_000,
            end_ns: 1_784_064_060_000_000_000,
            filters: BTreeMap::new(),
            group_by: vec!["service".into()],
            measures: vec!["count".into()],
        };
        let spilled_rows = archive.query(&query).unwrap().rows;
        assert_eq!(spilled_rows.len(), 1);
        let no_spill_output = dir.path().join("no-spill.tfold");
        encode(
            &input,
            &contract,
            &no_spill_output,
            &EncodeOptions::default(),
        )
        .unwrap();
        assert_eq!(
            spilled_rows,
            Archive::open(no_spill_output)
                .unwrap()
                .query(&query)
                .unwrap()
                .rows
        );
    }

    #[test]
    fn repeated_encodes_are_byte_deterministic_and_tampering_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("events.jsonl");
        let mut writer = File::create(&input).unwrap();
        generate(
            &GeneratorConfig {
                scenario: Scenario::Standard,
                events: Some(200),
                max_output_bytes: None,
                seed: 19,
            },
            &mut writer,
        )
        .unwrap();
        writer.flush().unwrap();
        let contract =
            Contract::parse(include_str!("../../../contracts/telemetry-v1.toml").to_owned())
                .unwrap();
        let first_path = dir.path().join("first.tfold");
        let second_path = dir.path().join("second.tfold");
        let first = encode(&input, &contract, &first_path, &EncodeOptions::default()).unwrap();
        let second = encode(&input, &contract, &second_path, &EncodeOptions::default()).unwrap();
        assert_eq!(first.archive_hash, second.archive_hash);
        assert_eq!(archive_files(&first_path), archive_files(&second_path));

        let view = first_path.join("views/view-00.tfv");
        File::options()
            .append(true)
            .open(view)
            .unwrap()
            .write_all(b"tampered")
            .unwrap();
        assert!(Archive::open(&first_path).unwrap().verify().is_err());
    }

    #[test]
    fn auto_layout_keeps_the_smaller_complete_archive() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("cross-product.jsonl");
        let mut writer = File::create(&input).unwrap();
        let mut index = 0_u64;
        for event_type in 0..24 {
            for operation in 0..24 {
                for model in 0..24 {
                    let event = CanonicalEvent {
                        schema_version: tracefold_core::CANONICAL_SCHEMA_VERSION,
                        timestamp_ns: 1_784_064_000_000_000_000,
                        event_id: format!("event-{index}"),
                        trace_id: None,
                        span_id: None,
                        parent_span_id: None,
                        service: "service".into(),
                        operation: Some(format!("operation-{operation}")),
                        event_type: format!("type-{event_type}"),
                        severity: tracefold_core::Severity::Info,
                        status: tracefold_core::Status::Ok,
                        error_code: None,
                        model: Some(format!("model-{model}")),
                        duration_ns: Some(1),
                        bytes_in: Some(1),
                        bytes_out: Some(1),
                        tokens_in: Some(1),
                        tokens_out: Some(1),
                        attributes: BTreeMap::new(),
                        body: serde_json::Value::Null,
                    };
                    writeln!(writer, "{}", event.canonical_line().unwrap()).unwrap();
                    index += 1;
                }
            }
        }
        writer.flush().unwrap();
        let contract = Contract::parse(
            include_str!("../../../contracts/telemetry-v1.toml")
                .replace("recent = \"24h\"", "recent = \"0\""),
        )
        .unwrap();
        let output = dir.path().join("auto.tfold");
        let encoded = encode(&input, &contract, &output, &EncodeOptions::default()).unwrap();
        assert_eq!(encoded.requested_layout, Layout::Auto);
        assert_eq!(
            encoded.selected_layout,
            Layout::Separate,
            "candidate sizes: {:?}",
            encoded.candidate_archive_bytes
        );
        assert!(
            encoded.candidate_archive_bytes["separate"]
                < encoded.candidate_archive_bytes["unified"]
        );
        let inspected = Archive::open(output).unwrap().inspect();
        assert_eq!(inspected.selected_layout, Layout::Separate);
    }

    fn archive_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, path: &Path, output: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, output);
                } else {
                    output.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut output = BTreeMap::new();
        visit(root, root, &mut output);
        output
    }
}
