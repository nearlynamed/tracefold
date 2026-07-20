use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tracefold_core::{
    CanonicalEvent, CellKey, CellState, Contract, Family, Measure, RecentRetention, ScalarValue,
    aggregate::measure_name,
};

use crate::{
    ARCHIVE_FORMAT_VERSION,
    codec::{EncodedRow, ViewSchema, write_view},
    raw_codec::RawEventWriter,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Auto,
    Separate,
    Unified,
}

#[derive(Debug, Clone)]
pub struct EncodeOptions {
    pub force: bool,
    pub layout: Layout,
    pub aggregation_budget_bytes: u64,
    pub cardinality_limit: usize,
    pub zstd_level: i32,
    pub git_commit: Option<String>,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            force: false,
            layout: Layout::Auto,
            aggregation_budget_bytes: 512 * 1024 * 1024,
            cardinality_limit: 10_000_000,
            zstd_level: 9,
            git_commit: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodeResult {
    pub schema_version: u16,
    pub output: PathBuf,
    pub archive_hash: String,
    pub source_hash: String,
    pub record_count: u64,
    pub error_count: u64,
    pub recent_count: u64,
    pub hot_cutoff_ns: i64,
    pub requested_layout: Layout,
    pub selected_layout: Layout,
    pub candidate_archive_bytes: BTreeMap<String, u64>,
    pub spill_count: u64,
    pub elapsed_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ArchiveMeta {
    pub archive_version: u16,
    pub contract_version: u16,
    pub canonical_schema_version: u16,
    pub encoder_version: String,
    pub git_commit: Option<String>,
    pub contract_hash: String,
    pub source_content_hash: String,
    pub archive_hash: String,
    pub record_count: u64,
    pub min_timestamp_ns: i64,
    pub max_timestamp_ns: i64,
    pub hot_cutoff_ns: i64,
    pub bucket_width_ns: i64,
    pub error_count: u64,
    pub recent_count: u64,
    pub requested_layout: Layout,
    pub layout: Layout,
    pub candidate_archive_bytes: BTreeMap<String, u64>,
    pub zstd_level: i32,
    pub components: BTreeMap<String, ComponentSize>,
    pub views: Vec<ViewMeta>,
    pub dictionary_cardinalities: BTreeMap<String, u64>,
    pub encode: EncodeStats,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComponentSize {
    pub logical_bytes: u64,
    pub compressed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ViewMeta {
    pub id: String,
    pub path: String,
    pub dimensions: Vec<String>,
    pub measures: Vec<Measure>,
    pub families: Vec<String>,
    pub family_measure_indices: BTreeMap<String, Vec<usize>>,
    pub row_count: u64,
    pub block_count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct EncodeStats {
    pub elapsed_ns: u64,
    pub pass_one_ns: u64,
    pub pass_two_ns: u64,
    pub peak_aggregation_estimate_bytes: u64,
    pub spill_count: u64,
    pub spill_bytes: u64,
    pub temporary_disk_high_water_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChecksumEntry {
    pub bytes: u64,
    pub blake3: String,
}

#[derive(Debug, Clone)]
struct PhysicalView {
    id: String,
    family: Family,
    families: Vec<String>,
    family_measure_indices: BTreeMap<String, Vec<usize>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpillRow {
    key: CellKey,
    state: CellState,
}

#[derive(Debug)]
struct PassOne {
    source_hash: String,
    record_count: u64,
    min_timestamp_ns: i64,
    max_timestamp_ns: i64,
    error_count: u64,
    dictionaries: BTreeMap<String, BTreeSet<String>>,
}

pub fn encode(
    input: &Path,
    contract: &Contract,
    output: &Path,
    options: &EncodeOptions,
) -> anyhow::Result<EncodeResult> {
    if options.layout == Layout::Auto {
        encode_auto(input, contract, output, options)
    } else {
        encode_explicit(input, contract, output, options)
    }
}

fn encode_auto(
    input: &Path,
    contract: &Contract,
    output: &Path,
    options: &EncodeOptions,
) -> anyhow::Result<EncodeResult> {
    let started = Instant::now();
    if output.exists() && !options.force {
        anyhow::bail!("output already exists; pass --force to replace it");
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let candidates = tempfile::Builder::new()
        .prefix(".tracefold-auto-")
        .tempdir_in(parent)?;
    let separate_path = candidates.path().join("separate.tfold");
    let mut separate_options = options.clone();
    separate_options.force = false;
    separate_options.layout = Layout::Separate;
    encode_explicit(input, contract, &separate_path, &separate_options)?;

    let unified_path = candidates.path().join("unified.tfold");
    let has_unified = unified_legal(contract);
    if has_unified {
        let mut unified_options = options.clone();
        unified_options.force = false;
        unified_options.layout = Layout::Unified;
        encode_explicit(input, contract, &unified_path, &unified_options)?;
    }

    let mut candidate_bytes =
        BTreeMap::from([("separate".to_owned(), archive_bytes(&separate_path)?)]);
    if has_unified {
        candidate_bytes.insert("unified".to_owned(), archive_bytes(&unified_path)?);
    }
    for _ in 0..4 {
        prepare_auto_candidate(&separate_path, &candidate_bytes)?;
        if has_unified {
            prepare_auto_candidate(&unified_path, &candidate_bytes)?;
        }
        let mut observed =
            BTreeMap::from([("separate".to_owned(), archive_bytes(&separate_path)?)]);
        if has_unified {
            observed.insert("unified".to_owned(), archive_bytes(&unified_path)?);
        }
        if observed == candidate_bytes {
            break;
        }
        candidate_bytes = observed;
    }

    let selected_layout = if candidate_bytes
        .get("unified")
        .is_some_and(|unified| unified < &candidate_bytes["separate"])
    {
        Layout::Unified
    } else {
        Layout::Separate
    };
    let selected_path = match selected_layout {
        Layout::Separate => &separate_path,
        Layout::Unified => &unified_path,
        Layout::Auto => unreachable!(),
    };
    let meta: ArchiveMeta = serde_json::from_reader(File::open(selected_path.join("meta.json"))?)?;
    crate::Archive::open(selected_path)?.verify()?;
    publish_existing(selected_path, output, options.force)?;
    Ok(EncodeResult {
        schema_version: 1,
        output: output.to_owned(),
        archive_hash: meta.archive_hash,
        source_hash: meta.source_content_hash,
        record_count: meta.record_count,
        error_count: meta.error_count,
        recent_count: meta.recent_count,
        hot_cutoff_ns: meta.hot_cutoff_ns,
        requested_layout: Layout::Auto,
        selected_layout,
        candidate_archive_bytes: candidate_bytes,
        spill_count: meta.encode.spill_count,
        elapsed_ns: started.elapsed().as_nanos() as u64,
    })
}

fn encode_explicit(
    input: &Path,
    contract: &Contract,
    output: &Path,
    options: &EncodeOptions,
) -> anyhow::Result<EncodeResult> {
    let started = Instant::now();
    if options.layout == Layout::Auto {
        anyhow::bail!("internal error: explicit encoder received auto layout");
    }
    if output.exists() && !options.force {
        anyhow::bail!("output already exists; pass --force to replace it");
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = tempfile::Builder::new()
        .prefix(".tracefold-encode-")
        .tempdir_in(parent)?;
    let archive_root = temp.path();
    fs::create_dir_all(archive_root.join("dictionaries"))?;
    fs::create_dir_all(archive_root.join("views"))?;
    fs::create_dir_all(archive_root.join("raw"))?;
    let spills = tempfile::Builder::new()
        .prefix("tracefold-spills-")
        .tempdir_in(parent)?;

    let physical_views = physical_views(contract, options.layout)?;
    let pass_one = pass_one(input, contract, options.cardinality_limit)?;
    if pass_one.record_count == 0 {
        anyhow::bail!("input contains no canonical events");
    }
    let hot_cutoff_ns = match contract.recent_retention()? {
        RecentRetention::All => pass_one.min_timestamp_ns,
        RecentRetention::Duration(duration) => pass_one.max_timestamp_ns.saturating_sub(duration),
    };
    let dictionary_maps =
        write_dictionaries(archive_root, &pass_one.dictionaries, options.zstd_level)?;
    fs::write(archive_root.join("contract.toml"), &contract.source)?;

    let mut cells: Vec<BTreeMap<CellKey, CellState>> =
        physical_views.iter().map(|_| BTreeMap::new()).collect();
    let mut spill_files: Vec<Vec<PathBuf>> = physical_views.iter().map(|_| Vec::new()).collect();
    let mut stats = EncodeStats::default();
    let mut recent =
        RawEventWriter::create(archive_root.join("raw/recent.tfr"), options.zstd_level)?;
    let mut errors =
        RawEventWriter::create(archive_root.join("raw/errors.tfr"), options.zstd_level)?;
    let reader = BufReader::new(File::open(input)?);
    let bucket_width = contract.bucket_ns()?;
    let mut recent_count = 0_u64;
    for line in reader.lines() {
        let event = CanonicalEvent::parse_line(&line?)?;
        let is_error = is_error(contract, &event);
        if is_error {
            errors.push(event.clone())?;
        } else if event.timestamp_ns >= hot_cutoff_ns {
            recent.push(event.clone())?;
            recent_count = recent_count
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("recent record count overflow"))?;
        }
        for (index, view) in physical_views.iter().enumerate() {
            let key = CellKey {
                bucket_ns: event.timestamp_ns.div_euclid(bucket_width) * bucket_width,
                dimensions: view
                    .family
                    .dimensions
                    .iter()
                    .map(|field| {
                        event.dimension(field).map_or(ScalarValue::Null, |value| {
                            let id = dictionary_maps[field][value];
                            ScalarValue::String(id.to_string())
                        })
                    })
                    .collect(),
            };
            cells[index]
                .entry(key)
                .or_insert_with(|| CellState::new(&view.family))
                .update(&view.family, &event)?;
        }
        let estimate = estimated_cells_bytes(&cells, &physical_views);
        stats.peak_aggregation_estimate_bytes = stats.peak_aggregation_estimate_bytes.max(estimate);
        if estimate > options.aggregation_budget_bytes {
            spill_all(
                &mut cells,
                &physical_views,
                &mut spill_files,
                &spills,
                &mut stats,
            )?;
        }
    }
    recent.finish()?;
    errors.finish()?;

    let mut view_meta = Vec::new();
    for (index, view) in physical_views.iter().enumerate() {
        for spill in &spill_files[index] {
            merge_spill(&mut cells[index], &view.family, spill)?;
        }
        let row_count = cells[index].len() as u64;
        let rows = std::mem::take(&mut cells[index])
            .into_iter()
            .map(|(key, state)| EncodedRow { key, state });
        let path = format!("views/{}.tfv", view.id);
        let indexes = write_view(
            &archive_root.join(&path),
            bucket_width,
            &ViewSchema {
                dimensions: view.family.dimensions.clone(),
                measures: view.family.measures.clone(),
            },
            rows,
            options.zstd_level,
        )?;
        view_meta.push(ViewMeta {
            id: view.id.clone(),
            path,
            dimensions: view.family.dimensions.clone(),
            measures: view.family.measures.clone(),
            families: view.families.clone(),
            family_measure_indices: view.family_measure_indices.clone(),
            row_count,
            block_count: indexes.len() as u32,
        });
    }

    let dictionary_cardinalities = pass_one
        .dictionaries
        .iter()
        .map(|(field, values)| (field.clone(), values.len() as u64 + 1))
        .collect();
    let mut meta = ArchiveMeta {
        archive_version: ARCHIVE_FORMAT_VERSION,
        contract_version: contract.version,
        canonical_schema_version: 1,
        encoder_version: env!("CARGO_PKG_VERSION").into(),
        git_commit: options.git_commit.clone(),
        contract_hash: contract.hash().to_hex().to_string(),
        source_content_hash: pass_one.source_hash.clone(),
        archive_hash: String::new(),
        record_count: pass_one.record_count,
        min_timestamp_ns: pass_one.min_timestamp_ns,
        max_timestamp_ns: pass_one.max_timestamp_ns,
        hot_cutoff_ns,
        bucket_width_ns: bucket_width,
        error_count: pass_one.error_count,
        recent_count,
        requested_layout: options.layout,
        layout: options.layout,
        candidate_archive_bytes: BTreeMap::new(),
        zstd_level: options.zstd_level,
        components: component_sizes(archive_root)?,
        views: view_meta,
        dictionary_cardinalities,
        encode: stats,
        warnings: vec![
            "Telemetry bodies and retained raw tiers may contain secrets; TraceFold does not redact them."
                .into(),
        ],
    };
    write_json(archive_root.join("meta.json"), &meta)?;
    let initial_checksums = checksums(archive_root)?;
    let archive_hash = hash_checksum_manifest(&initial_checksums)?;
    meta.archive_hash.clone_from(&archive_hash);
    write_json(archive_root.join("meta.json"), &meta)?;
    let checksums = checksums(archive_root)?;
    write_json(archive_root.join("checksums.json"), &checksums)?;

    crate::Archive::open(archive_root)?.verify()?;
    publish_temp(temp, output, options.force)?;
    Ok(EncodeResult {
        schema_version: 1,
        output: output.to_owned(),
        archive_hash,
        source_hash: pass_one.source_hash,
        record_count: pass_one.record_count,
        error_count: pass_one.error_count,
        recent_count,
        hot_cutoff_ns,
        requested_layout: options.layout,
        selected_layout: options.layout,
        candidate_archive_bytes: BTreeMap::new(),
        spill_count: meta.encode.spill_count,
        elapsed_ns: started.elapsed().as_nanos() as u64,
    })
}

fn pass_one(
    input: &Path,
    contract: &Contract,
    cardinality_limit: usize,
) -> anyhow::Result<PassOne> {
    let dimensions: BTreeSet<String> = contract
        .families
        .iter()
        .flat_map(|family| family.dimensions.iter().cloned())
        .collect();
    let mut dictionaries: BTreeMap<String, BTreeSet<String>> = dimensions
        .into_iter()
        .map(|field| (field, BTreeSet::new()))
        .collect();
    let mut ids = HashSet::new();
    let mut hasher = blake3::Hasher::new();
    let mut count = 0_u64;
    let mut errors = 0_u64;
    let mut min_timestamp = i64::MAX;
    let mut max_timestamp = i64::MIN;
    for line in BufReader::new(File::open(input)?).lines() {
        let event = CanonicalEvent::parse_line(&line?)?;
        if !ids.insert(event.event_id.clone()) {
            anyhow::bail!("duplicate event_id `{}`", event.event_id);
        }
        let canonical = event.canonical_line()?;
        hasher.update(canonical.as_bytes());
        hasher.update(b"\n");
        count = count
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("record count overflow"))?;
        min_timestamp = min_timestamp.min(event.timestamp_ns);
        max_timestamp = max_timestamp.max(event.timestamp_ns);
        errors += u64::from(is_error(contract, &event));
        for (field, values) in &mut dictionaries {
            if let Some(value) = event.dimension(field) {
                values.insert(value.to_owned());
                if values.len() > cardinality_limit {
                    anyhow::bail!(
                        "dimension `{field}` exceeds cardinality limit {cardinality_limit}"
                    );
                }
            }
        }
    }
    Ok(PassOne {
        source_hash: hasher.finalize().to_hex().to_string(),
        record_count: count,
        min_timestamp_ns: min_timestamp,
        max_timestamp_ns: max_timestamp,
        error_count: errors,
        dictionaries,
    })
}

fn physical_views(contract: &Contract, layout: Layout) -> anyhow::Result<Vec<PhysicalView>> {
    let groups: Vec<Vec<&Family>> = match layout {
        Layout::Auto => anyhow::bail!("auto layout must be resolved before view planning"),
        Layout::Separate => {
            let mut grouped: BTreeMap<Vec<String>, Vec<&Family>> = BTreeMap::new();
            for family in &contract.families {
                grouped
                    .entry(family.dimensions.clone())
                    .or_default()
                    .push(family);
            }
            grouped.into_values().collect()
        }
        Layout::Unified => {
            let dimensions: BTreeSet<String> = contract
                .families
                .iter()
                .flat_map(|family| family.dimensions.iter().cloned())
                .collect();
            if dimensions.len() > 8 {
                anyhow::bail!("unified layout has more than eight dimensions");
            }
            vec![contract.families.iter().collect()]
        }
    };
    groups
        .into_iter()
        .enumerate()
        .map(|(index, families)| {
            let dimensions = if layout == Layout::Unified {
                families
                    .iter()
                    .flat_map(|family| family.dimensions.iter().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            } else {
                families[0].dimensions.clone()
            };
            let mut measures = Vec::<Measure>::new();
            let mut measure_indexes = BTreeMap::<String, usize>::new();
            let mut family_measure_indices = BTreeMap::new();
            for family in &families {
                let mut indices = Vec::new();
                for measure in &family.measures {
                    let name = measure_name(measure);
                    if let Some(existing) = measure_indexes.get(&name) {
                        if measures[*existing] != *measure {
                            anyhow::bail!("incompatible duplicate measure `{name}`");
                        }
                        indices.push(*existing);
                    } else {
                        let position = measures.len();
                        measures.push(measure.clone());
                        measure_indexes.insert(name, position);
                        indices.push(position);
                    }
                }
                family_measure_indices.insert(family.name.clone(), indices);
            }
            let names = families.iter().map(|family| family.name.clone()).collect();
            Ok(PhysicalView {
                id: format!("view-{index:02}"),
                family: Family {
                    name: format!("physical-{index:02}"),
                    dimensions,
                    measures,
                },
                families: names,
                family_measure_indices,
            })
        })
        .collect()
}

fn unified_legal(contract: &Contract) -> bool {
    contract
        .families
        .iter()
        .flat_map(|family| &family.dimensions)
        .collect::<BTreeSet<_>>()
        .len()
        <= 8
}

fn write_dictionaries(
    root: &Path,
    dictionaries: &BTreeMap<String, BTreeSet<String>>,
    zstd_level: i32,
) -> anyhow::Result<BTreeMap<String, HashMap<String, u32>>> {
    let mut maps = BTreeMap::new();
    for (field, values) in dictionaries {
        let mut ordered = Vec::<Option<&str>>::with_capacity(values.len() + 1);
        ordered.push(None);
        ordered.extend(values.iter().map(|value| Some(value.as_str())));
        let file = File::create(
            root.join("dictionaries")
                .join(format!("{}.json.zst", safe_name(field))),
        )?;
        let mut encoder = zstd::stream::write::Encoder::new(file, zstd_level)?;
        serde_json::to_writer(&mut encoder, &ordered)?;
        encoder.finish()?.sync_all()?;
        maps.insert(
            field.clone(),
            values
                .iter()
                .enumerate()
                .map(|(index, value)| (value.clone(), index as u32 + 1))
                .collect(),
        );
    }
    Ok(maps)
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

pub(crate) fn is_error(contract: &Contract, event: &CanonicalEvent) -> bool {
    contract
        .retention
        .error_severities
        .iter()
        .any(|severity| severity == event.severity.as_str())
        || contract
            .retention
            .error_statuses
            .iter()
            .any(|status| status == event.status.as_str())
}

fn estimated_cells_bytes(cells: &[BTreeMap<CellKey, CellState>], views: &[PhysicalView]) -> u64 {
    cells
        .iter()
        .zip(views)
        .map(|(cells, view)| {
            cells.len() as u64
                * (64
                    + view.family.dimensions.len() as u64 * 16
                    + view.family.measures.len() as u64 * 64)
        })
        .sum()
}

fn spill_all(
    cells: &mut [BTreeMap<CellKey, CellState>],
    views: &[PhysicalView],
    spill_files: &mut [Vec<PathBuf>],
    spills: &TempDir,
    stats: &mut EncodeStats,
) -> anyhow::Result<()> {
    for (index, view_cells) in cells.iter_mut().enumerate() {
        if view_cells.is_empty() {
            continue;
        }
        let path = spills.path().join(format!(
            "{}-{:06}.jsonl",
            views[index].id,
            spill_files[index].len()
        ));
        let mut writer = BufWriter::new(File::create(&path)?);
        for (key, state) in std::mem::take(view_cells) {
            serde_json::to_writer(&mut writer, &SpillRow { key, state })?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        let bytes = fs::metadata(&path)?.len();
        stats.spill_count += 1;
        stats.spill_bytes += bytes;
        stats.temporary_disk_high_water_bytes = stats.spill_bytes;
        spill_files[index].push(path);
    }
    Ok(())
}

fn merge_spill(
    cells: &mut BTreeMap<CellKey, CellState>,
    family: &Family,
    path: &Path,
) -> anyhow::Result<()> {
    for line in BufReader::new(File::open(path)?).lines() {
        let row: SpillRow = serde_json::from_str(&line?)?;
        if let Some(existing) = cells.get_mut(&row.key) {
            existing.merge(&row.state, family)?;
        } else {
            cells.insert(row.key, row.state);
        }
    }
    Ok(())
}

fn component_sizes(root: &Path) -> anyhow::Result<BTreeMap<String, ComponentSize>> {
    let mut result = BTreeMap::<String, ComponentSize>::new();
    for path in files_recursive(root)? {
        let relative = path.strip_prefix(root)?.to_string_lossy();
        let component = if relative.starts_with("raw/recent") {
            "raw_recent"
        } else if relative.starts_with("raw/errors") {
            "raw_errors"
        } else {
            relative.split('/').next().unwrap_or("metadata")
        }
        .to_owned();
        let bytes = fs::metadata(&path)?.len();
        let entry = result.entry(component).or_default();
        entry.compressed_bytes += bytes;
        entry.logical_bytes += bytes;
    }
    Ok(result)
}

fn checksums(root: &Path) -> anyhow::Result<BTreeMap<String, ChecksumEntry>> {
    let mut result = BTreeMap::new();
    for path in files_recursive(root)? {
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "checksums.json" {
            continue;
        }
        let mut file = File::open(&path)?;
        let mut hasher = blake3::Hasher::new();
        let bytes = std::io::copy(&mut file, &mut hasher)?;
        result.insert(
            relative,
            ChecksumEntry {
                bytes,
                blake3: hasher.finalize().to_hex().to_string(),
            },
        );
    }
    Ok(result)
}

fn hash_checksum_manifest(entries: &BTreeMap<String, ChecksumEntry>) -> anyhow::Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(entries)?)
        .to_hex()
        .to_string())
}

fn archive_bytes(root: &Path) -> anyhow::Result<u64> {
    files_recursive(root)?
        .into_iter()
        .try_fold(0_u64, |total, path| {
            Ok(total.saturating_add(fs::metadata(path)?.len()))
        })
}

fn prepare_auto_candidate(
    root: &Path,
    candidate_archive_bytes: &BTreeMap<String, u64>,
) -> anyhow::Result<()> {
    let mut meta: ArchiveMeta = serde_json::from_reader(File::open(root.join("meta.json"))?)?;
    meta.requested_layout = Layout::Auto;
    meta.candidate_archive_bytes = candidate_archive_bytes.clone();
    meta.archive_hash.clear();
    write_json(root.join("meta.json"), &meta)?;
    let initial_checksums = checksums(root)?;
    meta.archive_hash = hash_checksum_manifest(&initial_checksums)?;
    write_json(root.join("meta.json"), &meta)?;
    write_json(root.join("checksums.json"), &checksums(root)?)?;
    crate::Archive::open(root)?.verify()?;
    Ok(())
}

fn files_recursive(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    fn visit(path: &Path, output: &mut Vec<PathBuf>) -> anyhow::Result<()> {
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(&path, output)?;
            } else if path.is_file() {
                output.push(path);
            }
        }
        Ok(())
    }
    let mut output = Vec::new();
    visit(root, &mut output)?;
    output.sort();
    Ok(output)
}

fn write_json(path: PathBuf, value: &impl Serialize) -> anyhow::Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn publish_temp(temp: TempDir, output: &Path, force: bool) -> anyhow::Result<()> {
    let temp_path = temp.keep();
    let backup = output.with_extension("tfold.replaced");
    if output.exists() {
        if !force {
            anyhow::bail!("output already exists");
        }
        if backup.exists() {
            fs::remove_dir_all(&backup)?;
        }
        fs::rename(output, &backup)?;
    }
    match fs::rename(&temp_path, output) {
        Ok(()) => {
            if backup.exists() {
                fs::remove_dir_all(backup)?;
            }
            Ok(())
        }
        Err(error) => {
            if backup.exists() && !output.exists() {
                let _ = fs::rename(&backup, output);
            }
            let _ = fs::remove_dir_all(temp_path);
            Err(error.into())
        }
    }
}

fn publish_existing(source: &Path, output: &Path, force: bool) -> anyhow::Result<()> {
    let backup = output.with_extension("tfold.replaced");
    if output.exists() {
        if !force {
            anyhow::bail!("output already exists");
        }
        if backup.exists() {
            fs::remove_dir_all(&backup)?;
        }
        fs::rename(output, &backup)?;
    }
    match fs::rename(source, output) {
        Ok(()) => {
            if backup.exists() {
                fs::remove_dir_all(backup)?;
            }
            Ok(())
        }
        Err(error) => {
            if backup.exists() && !output.exists() {
                let _ = fs::rename(&backup, output);
            }
            Err(error.into())
        }
    }
}
