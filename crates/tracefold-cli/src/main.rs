use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    str::FromStr,
};

use chrono::DateTime;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::json;
use tempfile::NamedTempFile;
use tracefold_archive::{Archive, EncodeOptions, Layout, RetainedClass, encode};
use tracefold_core::{
    Contract, QuerySpec, ScalarValue,
    generator::{GeneratorConfig, Scenario, generate},
    normalize::{Adapter, normalize_line},
};

#[derive(Debug, Parser)]
#[command(
    name = "tracefold",
    version,
    about = "Query-preserving telemetry archives"
)]
struct Cli {
    /// Emit versioned machine-readable output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Generate(GenerateArgs),
    Normalize(NormalizeArgs),
    Encode(EncodeArgs),
    Inspect(ArchivePath),
    Query(QueryArgs),
    Events(EventsArgs),
    Verify(VerifyArgs),
    Bench(BenchArgs),
}

#[derive(Debug, Args)]
struct GenerateArgs {
    #[arg(long)]
    scenario: String,
    #[arg(long, conflicts_with = "max_output_bytes")]
    events: Option<u64>,
    #[arg(long, conflicts_with = "events")]
    max_output_bytes: Option<u64>,
    #[arg(long)]
    seed: u64,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct NormalizeArgs {
    #[arg(long)]
    adapter: String,
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LayoutArg {
    Auto,
    Separate,
    Unified,
}

#[derive(Debug, Args)]
struct EncodeArgs {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    contract: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, value_enum, default_value_t = LayoutArg::Auto)]
    layout: LayoutArg,
    #[arg(long, default_value_t = 536_870_912)]
    aggregation_budget: u64,
    #[arg(long, default_value_t = 10_000_000)]
    cardinality_limit: usize,
    #[arg(long, default_value_t = 9)]
    zstd_level: i32,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct ArchivePath {
    archive: PathBuf,
}

#[derive(Debug, Args)]
struct QueryArgs {
    archive: PathBuf,
    #[arg(long)]
    family: String,
    #[arg(long)]
    start: String,
    #[arg(long)]
    end: String,
    #[arg(long = "where")]
    filters: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    group_by: Vec<String>,
    #[arg(long = "measure", value_delimiter = ',')]
    measures: Vec<String>,
}

#[derive(Debug, Args)]
struct EventsArgs {
    archive: PathBuf,
    #[arg(long, default_value = "all")]
    retained: String,
    #[arg(long = "where")]
    filters: Vec<String>,
    #[arg(long)]
    jsonl: bool,
}

#[derive(Debug, Args)]
struct VerifyArgs {
    archive: PathBuf,
    #[arg(long, requires = "queries")]
    source: Option<PathBuf>,
    #[arg(long, requires = "source")]
    queries: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BenchArgs {
    #[command(subcommand)]
    command: BenchCommand,
}

#[derive(Debug, Subcommand)]
enum BenchCommand {
    Fetch(BenchFetchArgs),
    Canonical(BenchCanonicalArgs),
    Smoke(BenchRunArgs),
    Synthetic(BenchRunArgs),
    Public(BenchRunArgs),
}

#[derive(Debug, Args)]
struct BenchCanonicalArgs {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    dataset: String,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    source_bytes: Option<u64>,
    #[arg(long)]
    extracted_bytes: Option<u64>,
    #[arg(long, default_value_t = tracefold_bench::DEFAULT_MAX_SOURCE_BYTES)]
    max_source_bytes: u64,
}

#[derive(Debug, Args)]
struct BenchFetchArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long, default_value_t = tracefold_bench::DEFAULT_MAX_SOURCE_BYTES)]
    max_source_bytes: u64,
}

#[derive(Debug, Args)]
struct BenchRunArgs {
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = tracefold_bench::DEFAULT_MAX_SOURCE_BYTES)]
    max_source_bytes: u64,
}

#[derive(Debug)]
struct CommandError {
    code: u8,
    error: anyhow::Error,
}

impl CommandError {
    fn new(code: u8, error: impl Into<anyhow::Error>) -> Self {
        Self {
            code,
            error: error.into(),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if cli.json {
                let payload = json!({
                    "schema_version": 1,
                    "success": false,
                    "exit_code": error.code,
                    "error": format!("{:#}", error.error),
                });
                println!("{}", serde_json::to_string(&payload).unwrap());
            } else {
                eprintln!("error: {:#}", error.error);
            }
            ExitCode::from(error.code)
        }
    }
}

fn run(cli: &Cli) -> Result<(), CommandError> {
    match &cli.command {
        Command::Generate(args) => run_generate(args).map_err(|error| CommandError::new(3, error)),
        Command::Normalize(args) => {
            run_normalize(args).map_err(|error| CommandError::new(3, error))
        }
        Command::Encode(args) => run_encode(args).map_err(|error| CommandError::new(3, error)),
        Command::Inspect(args) => {
            let archive =
                Archive::open(&args.archive).map_err(|error| CommandError::new(4, error))?;
            print_json(&archive.inspect()).map_err(|error| CommandError::new(4, error))
        }
        Command::Query(args) => run_query(args).map_err(|error| CommandError::new(5, error)),
        Command::Events(args) => run_events(args).map_err(|error| CommandError::new(5, error)),
        Command::Verify(args) => run_verify(args).map_err(|error| {
            let code = if format!("{error:#}").contains("mismatch") {
                6
            } else {
                4
            };
            CommandError::new(code, error)
        }),
        Command::Bench(args) => run_bench(args).map_err(|error| CommandError::new(7, error)),
    }
}

fn run_generate(args: &GenerateArgs) -> anyhow::Result<()> {
    ensure_parent(&args.output)?;
    let parent = args.output.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)?;
    let metadata = generate(
        &GeneratorConfig {
            scenario: Scenario::from_str(&args.scenario)?,
            events: args.events,
            max_output_bytes: args.max_output_bytes,
            seed: args.seed,
        },
        &mut temp,
    )?;
    temp.as_file().sync_all()?;
    temp.persist(&args.output)?;
    let metadata_path = PathBuf::from(format!("{}.meta.json", args.output.display()));
    write_json_atomic(&metadata_path, &metadata)?;
    print_json(&metadata)
}

fn run_normalize(args: &NormalizeArgs) -> anyhow::Result<()> {
    let adapter = Adapter::from_str(&args.adapter).map_err(anyhow::Error::msg)?;
    ensure_parent(&args.output)?;
    let parent = args.output.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)?;
    let mut count = 0_u64;
    let mut warnings = 0_u64;
    let mut hasher = blake3::Hasher::new();
    for (index, line) in BufReader::new(File::open(&args.input)?).lines().enumerate() {
        let (event, warning) = normalize_line(adapter, &line?, index as u64);
        let canonical = event.canonical_line()?;
        writeln!(temp, "{canonical}")?;
        hasher.update(canonical.as_bytes());
        hasher.update(b"\n");
        count += 1;
        warnings += u64::from(warning);
    }
    if count == 0 {
        anyhow::bail!("input is empty");
    }
    if warnings.saturating_mul(1000) > count {
        anyhow::bail!(
            "unparsed line rate {:.4}% exceeds 0.1%",
            warnings as f64 * 100.0 / count as f64
        );
    }
    temp.as_file().sync_all()?;
    temp.persist(&args.output)?;
    let metadata = json!({
        "schema_version": 1,
        "adapter": args.adapter,
        "record_count": count,
        "parse_warning_count": warnings,
        "timezone": "UTC",
        "timestamp_formats": match adapter {
            Adapter::LoghubZookeeper => vec!["%Y-%m-%d %H:%M:%S,%3f"],
            Adapter::LoghubBgl => vec!["unix-seconds", "%Y-%m-%d-%H.%M.%S.%f"],
        },
        "blake3": hasher.finalize().to_hex().to_string(),
    });
    write_json_atomic(
        &PathBuf::from(format!("{}.meta.json", args.output.display())),
        &metadata,
    )?;
    print_json(&metadata)
}

fn run_encode(args: &EncodeArgs) -> anyhow::Result<()> {
    eprintln!("warning: retained telemetry may contain secrets; TraceFold does not redact it");
    let contract = Contract::load(&args.contract)?;
    let result = encode(
        &args.input,
        &contract,
        &args.output,
        &EncodeOptions {
            force: args.force,
            layout: match args.layout {
                LayoutArg::Auto => Layout::Auto,
                LayoutArg::Separate => Layout::Separate,
                LayoutArg::Unified => Layout::Unified,
            },
            aggregation_budget_bytes: args.aggregation_budget,
            cardinality_limit: args.cardinality_limit,
            zstd_level: args.zstd_level,
            git_commit: git_commit(),
        },
    )?;
    print_json(&result)
}

fn run_query(args: &QueryArgs) -> anyhow::Result<()> {
    let archive = Archive::open(&args.archive)?;
    let result = archive.query(&QuerySpec {
        family: args.family.clone(),
        start_ns: parse_time(&args.start)?,
        end_ns: parse_time(&args.end)?,
        filters: parse_filters(&args.filters)?,
        group_by: args.group_by.clone(),
        measures: args.measures.clone(),
    })?;
    print_json(&result)
}

fn run_events(args: &EventsArgs) -> anyhow::Result<()> {
    let archive = Archive::open(&args.archive)?;
    let class = RetainedClass::from_str(&args.retained)?;
    let filters = parse_filters(&args.filters)?;
    let events: Vec<_> = archive
        .retained_events(class)?
        .into_iter()
        .filter(|event| {
            filters.iter().all(|(field, allowed)| {
                allowed.contains(&ScalarValue::from_option(event.dimension(field)))
            })
        })
        .collect();
    if args.jsonl {
        for event in &events {
            println!("{}", event.canonical_line()?);
        }
    } else {
        print_json(&json!({
            "schema_version": 1,
            "retention_class": args.retained,
            "hot_cutoff_ns": archive.inspect().hot_cutoff_ns,
            "record_count": events.len(),
            "events": events,
        }))?;
    }
    Ok(())
}

fn run_verify(args: &VerifyArgs) -> anyhow::Result<()> {
    let archive = Archive::open(&args.archive)?;
    let report = if let (Some(source), Some(queries)) = (&args.source, &args.queries) {
        let queries = BufReader::new(File::open(queries)?)
            .lines()
            .map(|line| Ok(serde_json::from_str::<QuerySpec>(&line?)?))
            .collect::<anyhow::Result<Vec<_>>>()?;
        archive.verify_queries(source, &queries)?
    } else {
        archive.verify()?
    };
    print_json(&report)
}

fn run_bench(args: &BenchArgs) -> anyhow::Result<()> {
    match &args.command {
        BenchCommand::Fetch(args) => print_json(&tracefold_bench::fetch(
            &args.manifest,
            args.max_source_bytes,
        )?),
        BenchCommand::Canonical(args) => print_json(&tracefold_bench::canonical(
            &args.output,
            &args.input,
            &args.dataset,
            args.source_bytes,
            args.extracted_bytes,
            args.max_source_bytes,
        )?),
        BenchCommand::Smoke(args) => print_json(&tracefold_bench::smoke(
            &args.output,
            args.max_source_bytes,
        )?),
        BenchCommand::Synthetic(args) => print_json(&tracefold_bench::synthetic(
            &args.output,
            args.max_source_bytes,
        )?),
        BenchCommand::Public(args) => print_json(&tracefold_bench::public(
            &args.output,
            args.max_source_bytes,
        )?),
    }
}

fn parse_time(value: &str) -> anyhow::Result<i64> {
    if let Ok(value) = value.parse() {
        return Ok(value);
    }
    DateTime::parse_from_rfc3339(value)?
        .timestamp_nanos_opt()
        .ok_or_else(|| anyhow::anyhow!("timestamp is outside nanosecond range"))
}

fn parse_filters(values: &[String]) -> anyhow::Result<BTreeMap<String, Vec<ScalarValue>>> {
    let mut result = BTreeMap::new();
    for value in values {
        let (field, values) = value
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("filter must be FIELD=VALUE[,VALUE]"))?;
        if field.is_empty() || values.is_empty() || result.contains_key(field) {
            anyhow::bail!("invalid or duplicate filter `{value}`");
        }
        result.insert(
            field.to_owned(),
            values
                .split(',')
                .map(|value| {
                    if value == "null" {
                        ScalarValue::Null
                    } else {
                        ScalarValue::String(value.to_owned())
                    }
                })
                .collect(),
        );
    }
    Ok(result)
}

fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))?;
    Ok(())
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    ensure_parent(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

fn print_json(value: &impl Serialize) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn git_commit() -> Option<String> {
    option_env!("TRACEFOLD_GIT_COMMIT").map(str::to_owned)
}
