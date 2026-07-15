//! Reproducible benchmark acquisition and orchestration.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use chrono::Utc;
use flate2::{Compression, write::GzEncoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tracefold_archive::{Archive, EncodeOptions, encode};
use tracefold_core::{
    Contract, OracleIndex, QuerySpec, ScalarValue,
    aggregate::measure_name,
    generator::{GeneratorConfig, Scenario, generate},
};

pub const DEFAULT_MAX_SOURCE_BYTES: u64 = 1_073_741_824;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchSummary {
    pub schema_version: u16,
    pub max_source_bytes: u64,
    pub corpora: Vec<FetchRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchRow {
    pub name: String,
    pub success: bool,
    pub source_path: Option<PathBuf>,
    pub raw_path: Option<PathBuf>,
    pub source_bytes: Option<u64>,
    pub raw_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub failure_kind: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSummary {
    pub schema_version: u16,
    pub output: PathBuf,
    pub attempts: u64,
    pub successes: u64,
    pub failures: u64,
    pub max_source_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkRow {
    pub schema_version: u16,
    pub run_id: String,
    pub timestamp: String,
    pub git_commit: Option<String>,
    pub command: String,
    pub host: HostMetadata,
    pub dataset: String,
    pub scenario: Option<String>,
    pub seed: Option<u64>,
    pub source_hash: Option<String>,
    pub record_count: Option<u64>,
    pub source_bytes: Option<u64>,
    pub extracted_bytes: Option<u64>,
    pub normalized_bytes: Option<u64>,
    pub size_limit_basis: String,
    pub size_limit_bytes: u64,
    pub source_within_limit: bool,
    pub contract_hash: Option<String>,
    pub archive_hash: Option<String>,
    pub bucket_width_ns: Option<i64>,
    pub retention: Option<String>,
    pub layout: Option<String>,
    pub baseline: String,
    pub codec: Option<String>,
    pub trial: u32,
    pub randomized_order: u32,
    pub archive_bytes: Option<u64>,
    pub compression_ratio: Option<f64>,
    pub bytes_per_event: Option<f64>,
    pub encode_wall_ns: Option<u64>,
    pub throughput_mib_s: Option<f64>,
    pub query_batch_wall_ns: Option<u64>,
    pub timing_mode: Option<String>,
    pub query_count: u64,
    pub query_workload_hash: Option<String>,
    pub query_workload_generated_at: Option<String>,
    pub query_workload: Option<Vec<QuerySpec>>,
    pub illegal_query_workload: Option<Vec<QuerySpec>>,
    pub oracle_result_sha256: Option<Vec<String>>,
    pub semantic_mismatch_count: u64,
    pub explicit_rejection_count: u64,
    pub raw_recoverability: Option<f64>,
    pub spill_count: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
    pub success: bool,
    pub failure_kind: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMetadata {
    pub os: String,
    pub arch: String,
    pub cpu_model: Option<String>,
    pub logical_cores: Option<u64>,
    pub total_memory_bytes: Option<u64>,
    pub filesystem: Option<String>,
    pub environment: String,
}

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    version: u16,
    max_source_bytes: u64,
    corpora: Vec<Corpus>,
}

#[derive(Debug, Deserialize)]
struct Corpus {
    name: String,
    source_url: String,
    archive_type: String,
    inner_path: String,
    expected_source_bytes: u64,
    expected_raw_bytes: u64,
    sha256: String,
}

pub fn fetch(manifest_path: &Path, requested_limit: u64) -> anyhow::Result<FetchSummary> {
    let manifest: CorpusManifest = toml::from_str(&fs::read_to_string(manifest_path)?)?;
    if manifest.version != 1 {
        anyhow::bail!("unsupported corpus manifest version {}", manifest.version);
    }
    let limit = requested_limit.min(manifest.max_source_bytes);
    let root = manifest_path
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    let data_dir = root.join("data/raw");
    fs::create_dir_all(&data_dir)?;
    let mut rows = Vec::new();
    for corpus in manifest.corpora {
        rows.push(fetch_one(&corpus, &data_dir, limit));
    }
    Ok(FetchSummary {
        schema_version: 1,
        max_source_bytes: limit,
        corpora: rows,
    })
}

fn fetch_one(corpus: &Corpus, data_dir: &Path, limit: u64) -> FetchRow {
    let result = (|| -> anyhow::Result<(PathBuf, PathBuf, u64, u64, String)> {
        if corpus.expected_source_bytes > limit {
            anyhow::bail!("source_size_limit");
        }
        let extension = if corpus.archive_type == "zip" {
            "zip"
        } else {
            "tar.gz"
        };
        let source_path = data_dir.join(format!("{}.{}", corpus.name, extension));
        let raw_path = data_dir.join(format!("{}.log", corpus.name));
        if !source_path.exists()
            || fs::metadata(&source_path)?.len() != corpus.expected_source_bytes
        {
            download_capped(&corpus.source_url, &source_path, limit)?;
        }
        let source_bytes = fs::metadata(&source_path)?.len();
        if source_bytes != corpus.expected_source_bytes {
            anyhow::bail!(
                "source length mismatch: expected {}, got {source_bytes}",
                corpus.expected_source_bytes
            );
        }
        let hash = sha256_file(&source_path)?;
        if hash != corpus.sha256 {
            anyhow::bail!("source SHA-256 mismatch");
        }
        extract(corpus, &source_path, &raw_path)?;
        let raw_bytes = fs::metadata(&raw_path)?.len();
        if raw_bytes != corpus.expected_raw_bytes {
            anyhow::bail!(
                "extracted length mismatch: expected {}, got {raw_bytes}",
                corpus.expected_raw_bytes
            );
        }
        Ok((source_path, raw_path, source_bytes, raw_bytes, hash))
    })();
    match result {
        Ok((source_path, raw_path, source_bytes, raw_bytes, hash)) => FetchRow {
            name: corpus.name.clone(),
            success: true,
            source_path: Some(source_path),
            raw_path: Some(raw_path),
            source_bytes: Some(source_bytes),
            raw_bytes: Some(raw_bytes),
            sha256: Some(hash),
            failure_kind: None,
            error: None,
        },
        Err(error) => FetchRow {
            name: corpus.name.clone(),
            success: false,
            source_path: None,
            raw_path: None,
            source_bytes: None,
            raw_bytes: None,
            sha256: None,
            failure_kind: Some(if error.to_string() == "source_size_limit" {
                "source_size_limit".into()
            } else {
                "acquisition".into()
            }),
            error: Some(format!("{error:#}")),
        },
    }
}

fn download_capped(url: &str, output: &Path, limit: u64) -> anyhow::Result<()> {
    let response = reqwest::blocking::Client::builder()
        .user_agent("tracefold/0.1 research artifact")
        .build()?
        .get(url)
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        anyhow::bail!("source_size_limit");
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)?;
    let mut reader = response;
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(read as u64);
        if bytes > limit {
            anyhow::bail!("source_size_limit");
        }
        temp.write_all(&buffer[..read])?;
    }
    temp.as_file().sync_all()?;
    temp.persist(output)?;
    Ok(())
}

fn extract(corpus: &Corpus, source: &Path, output: &Path) -> anyhow::Result<()> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)?;
    match corpus.archive_type.as_str() {
        "zip" => {
            let mut archive = zip::ZipArchive::new(File::open(source)?)?;
            let mut file = archive.by_name(&corpus.inner_path)?;
            std::io::copy(&mut file, &mut temp)?;
        }
        "tar_gz" => {
            let decoder = flate2::read::GzDecoder::new(File::open(source)?);
            let mut archive = tar::Archive::new(decoder);
            let mut found = false;
            for entry in archive.entries()? {
                let mut entry = entry?;
                if entry.path()?.as_ref() == Path::new(&corpus.inner_path) {
                    std::io::copy(&mut entry, &mut temp)?;
                    found = true;
                    break;
                }
            }
            if !found {
                anyhow::bail!("archive does not contain `{}`", corpus.inner_path);
            }
        }
        value => anyhow::bail!("unsupported archive type `{value}`"),
    }
    temp.as_file().sync_all()?;
    temp.persist(output)?;
    Ok(())
}

pub fn smoke(output: &Path, max_source_bytes: u64) -> anyhow::Result<BenchSummary> {
    run_synthetic_matrix(
        output,
        max_source_bytes,
        10_000,
        &[Scenario::Standard],
        "smoke",
    )
}

pub fn synthetic(output: &Path, max_source_bytes: u64) -> anyhow::Result<BenchSummary> {
    run_synthetic_matrix(
        output,
        max_source_bytes,
        1_000_000,
        &[
            Scenario::Standard,
            Scenario::LowCardinality,
            Scenario::HighCardinality,
            Scenario::HighEntropyBody,
            Scenario::ErrorBurst,
        ],
        "synthetic",
    )
}

pub fn public(output: &Path, max_source_bytes: u64) -> anyhow::Result<BenchSummary> {
    let root = workspace_root();
    let manifest: CorpusManifest =
        toml::from_str(&fs::read_to_string(root.join("benches/corpora.toml"))?)?;
    let mut rows = Vec::new();
    for corpus in manifest.corpora {
        let short_name = corpus.name.trim_start_matches("loghub-");
        let path = root.join(format!("data/normalized/{short_name}.jsonl"));
        if corpus.expected_source_bytes > max_source_bytes {
            rows.push(failure_row(
                &corpus.name,
                "public",
                max_source_bytes,
                "source_size_limit",
                format!(
                    "downloaded source is {} bytes",
                    corpus.expected_source_bytes
                ),
            ));
            continue;
        }
        if path.exists() {
            rows.extend(benchmark_dataset(
                &corpus.name,
                None,
                None,
                &path,
                corpus.expected_source_bytes,
                corpus.expected_raw_bytes,
                max_source_bytes,
                "public",
            )?);
        } else {
            rows.push(failure_row(
                &corpus.name,
                "public",
                max_source_bytes,
                "missing_normalized_data",
                format!(
                    "{} does not exist; run bench fetch and normalize",
                    path.display()
                ),
            ));
        }
    }
    write_rows(output, &rows)?;
    Ok(summary(output, max_source_bytes, &rows))
}

pub fn canonical(
    output: &Path,
    input: &Path,
    dataset: &str,
    source_bytes: Option<u64>,
    extracted_bytes: Option<u64>,
    max_source_bytes: u64,
) -> anyhow::Result<BenchSummary> {
    let normalized_bytes = fs::metadata(input)?.len();
    let source_bytes = source_bytes.unwrap_or(normalized_bytes);
    let extracted_bytes = extracted_bytes.unwrap_or(normalized_bytes);
    let rows = if source_bytes > max_source_bytes {
        vec![failure_row(
            dataset,
            "canonical",
            max_source_bytes,
            "source_size_limit",
            format!("declared source is {source_bytes} bytes"),
        )]
    } else {
        benchmark_dataset(
            dataset,
            None,
            None,
            input,
            source_bytes,
            extracted_bytes,
            max_source_bytes,
            "canonical",
        )?
    };
    write_rows(output, &rows)?;
    Ok(summary(output, max_source_bytes, &rows))
}

fn run_synthetic_matrix(
    output: &Path,
    max_source_bytes: u64,
    event_count: u64,
    scenarios: &[Scenario],
    command: &str,
) -> anyhow::Result<BenchSummary> {
    let temp = tempfile::tempdir()?;
    let mut rows = Vec::new();
    for scenario in scenarios {
        let path = temp.path().join(format!("{:?}.jsonl", scenario));
        let mut writer = BufWriter::new(File::create(&path)?);
        let metadata = generate(
            &GeneratorConfig {
                scenario: *scenario,
                events: Some(event_count),
                max_output_bytes: None,
                seed: 7,
            },
            &mut writer,
        )?;
        writer.flush()?;
        if metadata.bytes > max_source_bytes {
            rows.push(failure_row(
                &format!("synthetic-{:?}", scenario).to_lowercase(),
                command,
                max_source_bytes,
                "source_size_limit",
                format!("generated {} bytes", metadata.bytes),
            ));
            continue;
        }
        rows.extend(benchmark_dataset(
            &format!("synthetic-{:?}", scenario).to_lowercase(),
            Some(format!("{:?}", scenario).to_lowercase()),
            Some(7),
            &path,
            metadata.bytes,
            metadata.bytes,
            max_source_bytes,
            command,
        )?);
    }
    write_rows(output, &rows)?;
    Ok(summary(output, max_source_bytes, &rows))
}

#[allow(clippy::too_many_arguments)]
fn benchmark_dataset(
    dataset: &str,
    scenario: Option<String>,
    seed: Option<u64>,
    source: &Path,
    source_bytes: u64,
    extracted_bytes: u64,
    max_source_bytes: u64,
    command: &str,
) -> anyhow::Result<Vec<BenchmarkRow>> {
    let contract =
        Contract::parse(include_str!("../../../contracts/telemetry-v1.toml").to_owned())?;
    let (record_count, min_timestamp, max_timestamp, source_hash) = source_metadata(source)?;
    let normalized_bytes = fs::metadata(source)?.len();
    let host = host_metadata();
    let run_id = format!(
        "{}-{}-{}",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        dataset,
        &source_hash[..12]
    );
    let timestamp = Utc::now().to_rfc3339();
    let mut rows = Vec::new();
    let temp = tempfile::tempdir()?;
    let baselines = ["jsonl", "gzip-6", "zstd-3", "zstd-9"];
    let mut baseline_sources = BTreeMap::<String, PathBuf>::new();
    for (order, baseline) in baselines.iter().enumerate() {
        let started = Instant::now();
        let size = match *baseline {
            "jsonl" => {
                baseline_sources.insert((*baseline).into(), source.to_owned());
                normalized_bytes
            }
            "gzip-6" => {
                let path = temp.path().join("raw.jsonl.gz");
                let mut encoder = GzEncoder::new(File::create(&path)?, Compression::new(6));
                std::io::copy(&mut File::open(source)?, &mut encoder)?;
                encoder.finish()?.sync_all()?;
                let size = fs::metadata(&path)?.len();
                baseline_sources.insert((*baseline).into(), path);
                size
            }
            "zstd-3" | "zstd-9" => {
                let level = if *baseline == "zstd-3" { 3 } else { 9 };
                let path = temp.path().join(format!("raw-{level}.jsonl.zst"));
                let mut encoder = zstd::stream::write::Encoder::new(File::create(&path)?, level)?;
                std::io::copy(&mut File::open(source)?, &mut encoder)?;
                encoder.finish()?.sync_all()?;
                let size = fs::metadata(&path)?.len();
                baseline_sources.insert((*baseline).into(), path);
                size
            }
            _ => unreachable!(),
        };
        let elapsed = started.elapsed().as_nanos() as u64;
        rows.push(base_row(
            &run_id,
            &timestamp,
            command,
            &host,
            dataset,
            scenario.clone(),
            seed,
            &source_hash,
            record_count,
            source_bytes,
            extracted_bytes,
            normalized_bytes,
            max_source_bytes,
            &contract,
            baseline,
            order as u32,
            size,
            elapsed,
        ));
    }
    let archive_path = temp.path().join("archive.tfold");
    let started = Instant::now();
    let encoded = encode(
        source,
        &contract,
        &archive_path,
        &EncodeOptions {
            git_commit: git_commit(),
            ..EncodeOptions::default()
        },
    )?;
    let elapsed = started.elapsed().as_nanos() as u64;
    let archive_bytes = directory_size(&archive_path)?;
    let archive = Archive::open(&archive_path)?;
    let oracle = read_oracle_index(source, "jsonl", &contract)?;
    let queries = query_workload(&contract, min_timestamp, max_timestamp, &oracle)?;
    let illegal_queries = illegal_query_workload(&contract, min_timestamp, max_timestamp)?;
    let query_workload_generated_at = Utc::now().to_rfc3339();
    let query_workload_bytes = serde_json::to_vec(&queries)?;
    let query_workload_hash = blake3::hash(&query_workload_bytes).to_hex().to_string();
    let expected: Vec<_> = queries
        .iter()
        .map(|query| oracle.query(&contract, query))
        .collect::<Result<_, _>>()?;
    let oracle_result_sha256 = expected
        .iter()
        .map(|result| {
            let value = serde_json::to_value(&result.rows)?;
            Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&value)?)))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    for raw in &mut rows {
        let query_started = Instant::now();
        let baseline_oracle =
            read_oracle_index(&baseline_sources[&raw.baseline], &raw.baseline, &contract)?;
        let mut baseline_mismatches = 0_u64;
        for (query, expected) in queries.iter().zip(&expected) {
            let actual = baseline_oracle.query(&contract, query)?;
            baseline_mismatches += u64::from(actual.rows != expected.rows);
        }
        let baseline_rejections = illegal_queries
            .iter()
            .filter(|query| baseline_oracle.query(&contract, query).is_err())
            .count() as u64;
        raw.query_batch_wall_ns = Some(query_started.elapsed().as_nanos() as u64);
        raw.timing_mode = Some("single-process-decoded-batch".into());
        raw.query_count = queries.len() as u64;
        raw.query_workload_hash = Some(query_workload_hash.clone());
        raw.query_workload_generated_at = Some(query_workload_generated_at.clone());
        raw.semantic_mismatch_count = baseline_mismatches;
        raw.explicit_rejection_count = baseline_rejections;
        if baseline_mismatches > 0 || baseline_rejections != illegal_queries.len() as u64 {
            raw.success = false;
            raw.failure_kind = Some(if baseline_mismatches > 0 {
                "semantic_mismatch".into()
            } else {
                "contract_rejection".into()
            });
            raw.error = Some(format!(
                "{baseline_mismatches} query results differed; {baseline_rejections}/{} illegal queries rejected",
                illegal_queries.len()
            ));
        }
    }
    let query_started = Instant::now();
    let mut mismatches = 0_u64;
    for (query, expected) in queries.iter().zip(&expected) {
        let actual = archive.query(query)?;
        mismatches += u64::from(expected.rows != actual.rows);
    }
    let query_elapsed = query_started.elapsed().as_nanos() as u64;
    let explicit_rejections = illegal_queries
        .iter()
        .filter(|query| oracle.query(&contract, query).is_err() && archive.query(query).is_err())
        .count() as u64;
    let retained = archive
        .retained_events(tracefold_archive::RetainedClass::All)?
        .len() as u64;
    let mut row = base_row(
        &run_id,
        &timestamp,
        command,
        &host,
        dataset,
        scenario,
        seed,
        &source_hash,
        record_count,
        source_bytes,
        extracted_bytes,
        normalized_bytes,
        max_source_bytes,
        &contract,
        "tracefold-separate-zstd3",
        baselines.len() as u32,
        archive_bytes,
        elapsed,
    );
    row.archive_hash = Some(encoded.archive_hash);
    row.layout = Some("separate".into());
    row.codec = Some("zstd-3".into());
    row.query_batch_wall_ns = Some(query_elapsed);
    row.timing_mode = Some("single-process-warm".into());
    row.query_count = queries.len() as u64;
    row.query_workload_hash = Some(query_workload_hash);
    row.query_workload_generated_at = Some(query_workload_generated_at);
    row.query_workload = Some(queries);
    row.illegal_query_workload = Some(illegal_queries);
    row.oracle_result_sha256 = Some(oracle_result_sha256);
    row.semantic_mismatch_count = mismatches;
    row.explicit_rejection_count = explicit_rejections;
    row.raw_recoverability = Some(retained as f64 / record_count as f64);
    row.spill_count = Some(encoded.spill_count);
    row.success = mismatches == 0;
    if mismatches > 0 {
        row.failure_kind = Some("semantic_mismatch".into());
        row.error = Some(format!("{mismatches} query results differed"));
    }
    rows.push(row);
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn base_row(
    run_id: &str,
    timestamp: &str,
    command: &str,
    host: &HostMetadata,
    dataset: &str,
    scenario: Option<String>,
    seed: Option<u64>,
    source_hash: &str,
    record_count: u64,
    source_bytes: u64,
    extracted_bytes: u64,
    normalized_bytes: u64,
    max_source_bytes: u64,
    contract: &Contract,
    baseline: &str,
    order: u32,
    archive_bytes: u64,
    elapsed: u64,
) -> BenchmarkRow {
    BenchmarkRow {
        schema_version: 1,
        run_id: run_id.into(),
        timestamp: timestamp.into(),
        git_commit: git_commit(),
        command: command.into(),
        host: host.clone(),
        dataset: dataset.into(),
        scenario,
        seed,
        source_hash: Some(source_hash.into()),
        record_count: Some(record_count),
        source_bytes: Some(source_bytes),
        extracted_bytes: Some(extracted_bytes),
        normalized_bytes: Some(normalized_bytes),
        size_limit_basis: "downloaded_or_generated_source".into(),
        size_limit_bytes: max_source_bytes,
        source_within_limit: source_bytes <= max_source_bytes,
        contract_hash: Some(contract.hash().to_hex().to_string()),
        archive_hash: None,
        bucket_width_ns: contract.bucket_ns().ok(),
        retention: Some(contract.retention.recent.clone()),
        layout: None,
        baseline: baseline.into(),
        codec: None,
        trial: 0,
        randomized_order: order,
        archive_bytes: Some(archive_bytes),
        compression_ratio: Some(normalized_bytes as f64 / archive_bytes.max(1) as f64),
        bytes_per_event: Some(archive_bytes as f64 / record_count.max(1) as f64),
        encode_wall_ns: Some(elapsed),
        throughput_mib_s: Some(
            normalized_bytes as f64 / 1_048_576.0 / (elapsed.max(1) as f64 / 1_000_000_000.0),
        ),
        query_batch_wall_ns: None,
        timing_mode: None,
        query_count: 0,
        query_workload_hash: None,
        query_workload_generated_at: None,
        query_workload: None,
        illegal_query_workload: None,
        oracle_result_sha256: None,
        semantic_mismatch_count: 0,
        explicit_rejection_count: 0,
        raw_recoverability: Some(1.0),
        spill_count: None,
        peak_rss_bytes: None,
        success: true,
        failure_kind: None,
        error: None,
    }
}

fn failure_row(
    dataset: &str,
    command: &str,
    max_source_bytes: u64,
    kind: &str,
    error: String,
) -> BenchmarkRow {
    BenchmarkRow {
        schema_version: 1,
        run_id: format!("{}-{dataset}", Utc::now().format("%Y%m%dT%H%M%SZ")),
        timestamp: Utc::now().to_rfc3339(),
        git_commit: git_commit(),
        command: command.into(),
        host: host_metadata(),
        dataset: dataset.into(),
        scenario: None,
        seed: None,
        source_hash: None,
        record_count: None,
        source_bytes: None,
        extracted_bytes: None,
        normalized_bytes: None,
        size_limit_basis: "downloaded_or_generated_source".into(),
        size_limit_bytes: max_source_bytes,
        source_within_limit: kind != "source_size_limit",
        contract_hash: None,
        archive_hash: None,
        bucket_width_ns: None,
        retention: None,
        layout: None,
        baseline: "none".into(),
        codec: None,
        trial: 0,
        randomized_order: 0,
        archive_bytes: None,
        compression_ratio: None,
        bytes_per_event: None,
        encode_wall_ns: None,
        throughput_mib_s: None,
        query_batch_wall_ns: None,
        timing_mode: None,
        query_count: 0,
        query_workload_hash: None,
        query_workload_generated_at: None,
        query_workload: None,
        illegal_query_workload: None,
        oracle_result_sha256: None,
        semantic_mismatch_count: 0,
        explicit_rejection_count: 0,
        raw_recoverability: None,
        spill_count: None,
        peak_rss_bytes: None,
        success: false,
        failure_kind: Some(kind.into()),
        error: Some(error),
    }
}

fn query_workload(
    contract: &Contract,
    min_timestamp: i64,
    max_timestamp: i64,
    oracle: &OracleIndex,
) -> anyhow::Result<Vec<QuerySpec>> {
    let width = contract.bucket_ns()?;
    let start = min_timestamp.div_euclid(width) * width;
    let end = max_timestamp
        .div_euclid(width)
        .checked_add(1)
        .and_then(|value| value.checked_mul(width))
        .ok_or_else(|| anyhow::anyhow!("query range overflow"))?;
    let window_buckets = [60_i64, 360, 1_440, 10_080, i64::MAX];
    let mut queries = Vec::new();
    for family in &contract.families {
        let values: BTreeMap<_, Vec<_>> = family
            .dimensions
            .iter()
            .map(|dimension| {
                let values = oracle.dimension_values(contract, &family.name, dimension, 4);
                (dimension.clone(), values)
            })
            .collect();
        for index in 0..40_usize {
            let requested_buckets = window_buckets[index % window_buckets.len()];
            let query_start = if requested_buckets == i64::MAX {
                start
            } else {
                end.saturating_sub(requested_buckets.saturating_mul(width))
                    .max(start)
            };
            let mut filters = BTreeMap::new();
            match index / 10 {
                1 => add_filter(&mut filters, &family.dimensions[0], &values, false),
                2 => {
                    add_filter(&mut filters, &family.dimensions[0], &values, false);
                    let second = family.dimensions.get(1).unwrap_or(&family.dimensions[0]);
                    add_filter(&mut filters, second, &values, false);
                }
                3 => add_filter(&mut filters, &family.dimensions[0], &values, true),
                _ => {}
            }
            let group_count = 1 + index % family.dimensions.len();
            let available_measures: Vec<_> = family.measures.iter().map(measure_name).collect();
            queries.push(QuerySpec {
                family: family.name.clone(),
                start_ns: query_start,
                end_ns: end,
                filters,
                group_by: family
                    .dimensions
                    .iter()
                    .take(group_count)
                    .cloned()
                    .collect(),
                measures: if index % 3 == 0 {
                    available_measures.iter().take(1).cloned().collect()
                } else {
                    Vec::new()
                },
            });
        }
    }
    Ok(queries)
}

fn add_filter(
    filters: &mut BTreeMap<String, Vec<ScalarValue>>,
    dimension: &str,
    values: &BTreeMap<String, Vec<ScalarValue>>,
    use_in: bool,
) {
    if let Some(available) = values.get(dimension).filter(|values| !values.is_empty()) {
        filters.insert(
            dimension.to_owned(),
            available
                .iter()
                .take(if use_in { 2 } else { 1 })
                .cloned()
                .collect(),
        );
    }
}

fn illegal_query_workload(
    contract: &Contract,
    min_timestamp: i64,
    max_timestamp: i64,
) -> anyhow::Result<Vec<QuerySpec>> {
    let width = contract.bucket_ns()?;
    let start = min_timestamp.div_euclid(width) * width;
    let end = max_timestamp
        .div_euclid(width)
        .checked_add(1)
        .and_then(|value| value.checked_mul(width))
        .ok_or_else(|| anyhow::anyhow!("query range overflow"))?;
    let family = &contract.families[0];
    Ok((0..20)
        .map(|index| {
            let mut query = QuerySpec {
                family: family.name.clone(),
                start_ns: start,
                end_ns: end,
                filters: BTreeMap::new(),
                group_by: Vec::new(),
                measures: Vec::new(),
            };
            match index % 5 {
                0 => query.family = "undeclared-family".into(),
                1 => query.start_ns = start.saturating_add(1),
                2 => query.group_by.push("undeclared_dimension".into()),
                3 => {
                    query.filters.insert("service".into(), Vec::new());
                }
                _ => query.measures.push("undeclared:sum".into()),
            }
            query
        })
        .collect())
}

fn source_metadata(path: &Path) -> anyhow::Result<(u64, i64, i64, String)> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = String::new();
    let mut count = 0_u64;
    let mut min = i64::MAX;
    let mut max = i64::MIN;
    let mut hash = blake3::Hasher::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        hash.update(line.as_bytes());
        let event = tracefold_core::CanonicalEvent::parse_line(line.trim_end())?;
        count = count
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("record count overflow"))?;
        min = min.min(event.timestamp_ns);
        max = max.max(event.timestamp_ns);
    }
    if count == 0 {
        anyhow::bail!("dataset is empty");
    }
    Ok((count, min, max, hash.finalize().to_hex().to_string()))
}

fn read_oracle_index(
    path: &Path,
    baseline: &str,
    contract: &Contract,
) -> anyhow::Result<OracleIndex> {
    match baseline {
        "jsonl" => oracle_from_reader(BufReader::new(File::open(path)?), contract),
        "gzip-6" => oracle_from_reader(
            BufReader::new(flate2::read::GzDecoder::new(File::open(path)?)),
            contract,
        ),
        "zstd-3" | "zstd-9" => oracle_from_reader(
            BufReader::new(zstd::stream::read::Decoder::new(File::open(path)?)?),
            contract,
        ),
        _ => anyhow::bail!("unsupported raw baseline `{baseline}`"),
    }
}

fn oracle_from_reader(reader: impl BufRead, contract: &Contract) -> anyhow::Result<OracleIndex> {
    let mut oracle = OracleIndex::new(contract);
    for line in reader.lines() {
        let event = tracefold_core::CanonicalEvent::parse_line(&line?)?;
        oracle.ingest(contract, &event)?;
    }
    Ok(oracle)
}

fn write_rows(path: &Path, rows: &[BenchmarkRow]) -> anyhow::Result<()> {
    fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))?;
    let mut writer = BufWriter::new(File::create(path)?);
    for row in rows {
        serde_json::to_writer(&mut writer, row)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn summary(path: &Path, max_source_bytes: u64, rows: &[BenchmarkRow]) -> BenchSummary {
    BenchSummary {
        schema_version: 1,
        output: path.to_owned(),
        attempts: rows.len() as u64,
        successes: rows.iter().filter(|row| row.success).count() as u64,
        failures: rows.iter().filter(|row| !row.success).count() as u64,
        max_source_bytes,
    }
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn directory_size(path: &Path) -> anyhow::Result<u64> {
    fn visit(path: &Path, bytes: &mut u64) -> anyhow::Result<()> {
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(&path, bytes)?;
            } else {
                *bytes = bytes.saturating_add(fs::metadata(path)?.len());
            }
        }
        Ok(())
    }
    let mut bytes = 0;
    visit(path, &mut bytes)?;
    Ok(bytes)
}

fn host_metadata() -> HostMetadata {
    let cpu_model = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find_map(|line| line.strip_prefix("model name\t: ").map(str::to_owned))
        });
    let total_memory_bytes = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("MemTotal:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
                    .and_then(|value| value.checked_mul(1024))
            })
        });
    HostMetadata {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        cpu_model,
        logical_cores: std::thread::available_parallelism()
            .ok()
            .map(|value| value.get() as u64),
        total_memory_bytes,
        filesystem: None,
        environment: if fs::read_to_string("/proc/version")
            .is_ok_and(|version| version.to_lowercase().contains("microsoft"))
        {
            "wsl2".into()
        } else {
            "native-or-container".into()
        },
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_owned()
}

fn git_commit() -> Option<String> {
    option_env!("TRACEFOLD_GIT_COMMIT").map(str::to_owned)
}
