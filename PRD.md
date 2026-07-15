# TraceFold: Query-Preserving Compression for Tiered Telemetry Archives

**Status:** Implementation-ready product requirements and research plan  
**Working directory:** `/home/user/tracefold`  
**Primary deliverable:** Reproducible research artifact and technical paper  
**Secondary deliverable:** Usable local CLI  
**Implementation:** Rust 2024 core and CLI; Python 3.11+ reporting through `uv`  
**License:** MIT OR Apache-2.0  
**Archive format:** TraceFold Archive v1, stored as a `.tfold` directory

## 1. Handoff instructions

This document is the source of truth for implementation. A new coding-agent instance should read it completely, inspect the empty or partially implemented repository, and implement the full v1 without requesting further product decisions. Preserve these decisions unless a hard technical impossibility is demonstrated:

- Build a telemetry-first batch archive, not a streaming service.
- Preserve exact results for declared query families under explicit time-bucket semantics.
- Instantiate benchmark queries only after each archive is finalized.
- Retain exact canonical records for the newest 24 hours by default.
- Retain exact canonical error records for all time.
- Deliberately give up reconstruction of older successful raw events.
- Treat negative benchmark results as valid. Never tune away, omit, or hide regressions.
- Keep downloaded corpora and large generated data out of Git.
- Do not build the eventual hosted paper site in v1. Generate stable Markdown, SVG, and JSON artifacts that a later Next.js/Vercel site can consume.
- Do not modify or inspect `indexdataset` as part of this project.
- Do not push, deploy, or create external resources unless separately authorized.

## 2. Product summary

TraceFold tests a narrow systems thesis: telemetry archives can use materially less space and answer operational queries faster when they preserve a declared family of questions rather than every historical event byte.

A conventional compressor must reconstruct all input. TraceFold changes the contract:

1. Recent events remain exactly retrievable for a configurable hot window.
2. Error events remain exactly retrievable indefinitely.
3. Older successful events become query-sufficient aggregate views.
4. Declared aggregate query families remain exact for any legal runtime parameters.
5. Queries or raw retrieval outside the declared contract fail explicitly.

The main artifact is a rigorous comparison among raw-preserving storage, mature columnar/database formats, a semantic materialized-view baseline, and TraceFold's custom archive.

### Goals

- Define query preservation precisely enough to test automatically.
- Provide deterministic synthetic telemetry with controlled entropy, cardinality, error rate, and scale.
- Validate results on public Loghub data without requiring proprietary data.
- Produce Pareto curves for storage, query latency, ingestion cost, time precision, and raw recoverability.
- Make every published result reproducible from a corpus manifest, generator seed, contract, benchmark row, and host metadata.
- Produce an honest paper even if TraceFold fails to beat a strong semantic baseline.

### Non-goals

- General arbitrary JSONL schema inference.
- Arbitrary SQL, joins, regex predicates, or undeclared dimensions.
- Approximate query answers, sketches, approximate distinct counts, or quantiles in v1.
- Live ingestion, concurrent writers, background aging, or crash-safe streaming compaction.
- Full OTLP protobuf ingestion or an observability backend.
- Encryption, secret detection, or a claim that retained data is safe to share.
- Reconstructing old successful events.
- A hosted dashboard or Vercel application in v1.

## 3. Research questions and success definition

### Research questions

- **RQ1 — Storage:** How much smaller is a query-preserving archive than JSONL+Zstandard, raw Parquet, and DuckDB when recent and error events remain exact?
- **RQ2 — Retrieval:** What query-latency improvement or regression results from pre-aggregated query families and a block-indexed custom representation?
- **RQ3 — Semantic knobs:** How do hot-window duration and time-bucket width move the storage/fidelity frontier?
- **RQ4 — Workload shape:** How sensitive are results to scale, dimension cardinality, payload entropy, correlation, and error rate?
- **RQ5 — Encoding:** Does TraceFold's custom view encoding improve over storing the identical semantic views in Parquet with Zstandard?
- **RQ6 — Cost:** What ingestion throughput, peak-memory, and temporary-disk costs purchase the storage and query behavior?

### Completion versus favorable results

Implementation success does not require TraceFold to win. V1 is complete when:

- Every legal declared query returns exactly the same canonical result as the raw-data oracle.
- Every illegal or unavailable query fails clearly rather than returning a partial answer.
- The full benchmark matrix runs reproducibly and preserves failures.
- The paper reports all primary baselines, negative results, and threats to validity.

Performance hypotheses may be stated in the paper as hypotheses, not acceptance criteria:

- Query-preserving layouts should beat raw-preserving formats on storage when old payloads dominate.
- The advantage should shrink as the hot window or permanent error fraction grows.
- High-cardinality declared dimensions may make materialized views larger than raw columnar storage.
- A custom integer/dictionary view format may outperform view-equivalent Parquet, but this must be measured rather than assumed.

## 4. Semantic contract

### 4.1 Canonical event schema

The encoder accepts canonical newline-delimited JSON. Every line is one object with these fields:

```json
{
  "schema_version": 1,
  "timestamp_ns": 1784064000000000000,
  "event_id": "evt-000001",
  "trace_id": "trace-001",
  "span_id": "span-009",
  "parent_span_id": "span-004",
  "service": "checkout",
  "operation": "POST /checkout",
  "event_type": "request.completed",
  "severity": "INFO",
  "status": "OK",
  "error_code": null,
  "model": null,
  "duration_ns": 28400000,
  "bytes_in": 812,
  "bytes_out": 1940,
  "tokens_in": null,
  "tokens_out": null,
  "attributes": {
    "region": "us-east",
    "host": "checkout-3"
  },
  "body": {
    "message": "request completed"
  }
}
```

Rules:

- `schema_version`, `timestamp_ns`, `event_id`, `service`, `event_type`, `severity`, and `status` are required.
- `timestamp_ns` is signed Unix epoch nanoseconds. Input may be out of order.
- `event_id` must be unique within an input file; duplicates are rejected.
- `severity` is one of `TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`, `FATAL`, or `UNKNOWN`.
- `status` is one of `OK`, `ERROR`, or `UNSET`.
- Dimension-like fixed fields and every `attributes.*` value are nullable strings. Adapters stringify scalar upstream attributes deterministically and reject nested attribute values.
- Measures are nullable signed 64-bit integers. Floats are rejected; adapters must scale units before normalization.
- `body` may be any JSON value and is never used to answer aggregate query families.
- Unknown top-level fields are rejected so schema drift cannot silently change results.
- A normalizer emits compact, deterministically key-ordered JSONL. TraceFold guarantees exact canonical record recovery, not recovery of an upstream source line unless the adapter placed that line in `body`.

Aggregation semantics:

- `count` counts all records in the selected cell.
- `sum`, `min`, `max`, and fixed histograms ignore null measure values.
- Every numeric aggregate also stores `present_count`, allowing null behavior to be verified.
- Counts use `u64`; numeric accumulators use checked `i64`. Encoding fails on overflow.
- Null dimensions are represented as a real dictionary value and can be filtered with `field=null`.

### 4.2 Contract manifest

Encoding requires a versioned TOML manifest. The archive embeds the exact manifest and its BLAKE3 hash. The default manifest lives at `contracts/telemetry-v1.toml` and has this conceptual shape:

```toml
version = 1
name = "telemetry-v1"
time_bucket = "1m"

[retention]
recent = "24h"
error_severities = ["ERROR", "FATAL"]
error_statuses = ["ERROR"]

[[families]]
name = "event-volume"
dimensions = ["service", "event_type", "severity", "status"]
measures = [{ field = "*", op = "count" }]

[[families]]
name = "latency"
dimensions = ["service", "operation", "status"]
measures = [
  { field = "duration_ns", op = "count_present" },
  { field = "duration_ns", op = "sum" },
  { field = "duration_ns", op = "min" },
  { field = "duration_ns", op = "max" },
  { field = "duration_ns", op = "histogram", bounds = [1000000, 5000000, 10000000, 50000000, 100000000, 500000000, 1000000000] }
]

[[families]]
name = "traffic"
dimensions = ["service", "operation", "status"]
measures = [
  { field = "bytes_in", op = "sum" },
  { field = "bytes_out", op = "sum" }
]

[[families]]
name = "model-usage"
dimensions = ["service", "operation", "model", "status"]
measures = [
  { field = "tokens_in", op = "sum" },
  { field = "tokens_out", op = "sum" }
]

[[families]]
name = "error-volume"
dimensions = ["service", "operation", "error_code", "severity"]
measures = [{ field = "*", op = "count" }]
```

Validation rules:

- Durations accept integer units `ns`, `us`, `ms`, `s`, `m`, `h`, and `d` only.
- `time_bucket` must divide one day evenly and be between one second and one day.
- `retention.recent` accepts a duration, `0`, or `all`.
- A family may declare one to six dimensions.
- Dimensions must be fixed dimension fields or explicit `attributes.<key>` paths.
- Supported operations are exactly `count`, `count_present`, `sum`, `min`, `max`, and `histogram` with fixed integer bounds.
- Histogram bounds must be sorted and unique. Results include underflow and overflow bins.
- Duplicate names, unknown fields, unsupported operations, and incompatible duplicate measure definitions are errors.

### 4.3 Legal runtime query family

A manifest family preserves more than a finite answer. At query time, users may choose:

- Any `[start, end)` interval whose endpoints align exactly to the archive bucket width.
- Zero or more equality or `IN` filters on declared family dimensions.
- Any grouping that is a subset of the declared family dimensions.
- Any subset of the family's declared measures.

The time bucket is an implicit grouping key in every v1 aggregate query; v1 does not collapse an interval into one timeless total. Only buckets containing selected events are emitted, so zero-filling is a reporting/client concern. Results are exact under those rules. Rows are sorted by time bucket and then lexicographically by grouped dimensions. An empty selection returns an empty `rows` array, not an error.

V1 rejects:

- Unaligned time endpoints.
- Numeric range predicates other than time.
- Regex, substring, full-text, joins, ordering by measure, top-k, quantiles, distinct counts, undeclared fields, and cross-family combinations.
- Queries whose family is not embedded in the archive.

This rejection boundary is part of the measured semantic tradeoff and must be documented prominently.

### 4.4 Exact raw retention

The batch encoder first determines `max(timestamp_ns)`. With the default 24-hour policy:

```text
hot_cutoff = max_timestamp - 24 hours
```

- Every event at or after `hot_cutoff` is exactly retained.
- Every event classified as an error is exactly retained regardless of age.
- An event is an error when its normalized `severity` or `status` matches the manifest retention lists.
- To avoid duplication, `raw/errors.jsonl.zst` contains every error, while `raw/recent.jsonl.zst` contains only recent non-error events.
- `tracefold events --retained all` returns the union sorted by timestamp and event ID.
- `--retained recent` and `--retained errors` expose the two logical classes.
- Filters over retained raw events use fixed fields only and are evaluated by scanning the retained streams.
- The CLI never implies that retained raw results represent all historical events. Every response includes the hot cutoff and retention class.

`recent=0` retains only errors. `recent=all` retains every raw event and should converge toward the raw-preserving storage baselines.

## 5. Archive design

### 5.1 Physical layout

A `.tfold` archive is an immutable directory:

```text
archive.tfold/
  meta.json
  contract.toml
  checksums.json
  dictionaries/
    <dimension>.json.zst
  views/
    <view-id>.tfv
  raw/
    recent.jsonl.zst
    errors.jsonl.zst
```

`meta.json` includes:

- Archive, contract, and canonical schema versions.
- Encoder version and Git commit when available.
- Contract hash and source content hash.
- Record count, minimum/maximum timestamp, hot cutoff, bucket width, and error count.
- Per-component logical and compressed byte counts.
- View schemas and the families each view serves.
- Dictionary cardinalities.
- Encode timings, peak in-process aggregation estimate, spill count, and warnings.

`checksums.json` contains a BLAKE3 digest and byte length for every other archive file. `tracefold verify` checks paths, lengths, hashes, format bounds, dictionary references, sorted order, and aggregate invariants.

Encoding writes to a sibling temporary directory and atomically renames it only after verification succeeds. Existing output is never overwritten without `--force`; forced replacement still uses a temporary directory and rename. Temporary spills are cleaned on success, error, and normal interrupt handling.

### 5.2 Dictionaries and views

The primary `separate` layout creates one physical view for each unique dimension set. Families with identical dimensions share a view and store the union of their measures. It does not silently replace a family with a higher-dimensional superset.

An experimental `unified` layout creates one view using the union of all dimensions if that union contains at most eight dimensions. It is an ablation, not the default. If the union exceeds eight, the encoder rejects `--layout unified`.

Dictionary construction is deterministic:

- Pass one validates input, hashes canonical lines, finds timestamp bounds, checks event IDs, and collects every value used by a declared dimension.
- Values are sorted by UTF-8 byte order; ID zero is reserved for null; non-null IDs start at one.
- The encoder enforces a configurable cardinality ceiling, default ten million distinct values per dimension, and fails rather than silently hashing or truncating.

Pass two assigns dictionary IDs, routes old events into aggregate cells, and writes raw retained records. Recent and error events are **also included in aggregate views**, so aggregate queries cover the full dataset exactly.

### 5.3 Aggregate spill and merge

TraceFold must not require all aggregate cells to fit in memory.

- Maintain per-view hash maps up to a global configurable aggregation budget, default 512 MiB.
- When the estimated budget is exceeded, sort every non-empty map by `(bucket, dimension IDs)`, write deterministic uncompressed spill runs, clear the maps, and continue.
- At finalization, perform a k-way merge of all runs plus the remaining map, combining identical keys with checked arithmetic.
- Track peak estimated bytes, number and bytes of spills, and temporary-disk high-water mark.
- The publish benchmark uses the default budget. A small-budget test forces spills on tiny fixtures.

### 5.4 View binary format v1

Every `.tfv` file is little-endian and consists of:

1. Eight-byte magic `TFLDVIEW`.
2. `u16` format version `1`.
3. `i64` bucket width in nanoseconds.
4. Length-prefixed canonical JSON describing dimensions and measures.
5. `u32` block count.
6. A fixed-size block index.
7. Independently Zstandard-compressed data blocks.

Each block-index entry stores minimum bucket, exclusive maximum bucket, file offset, compressed length, uncompressed length, row count, and BLAKE3 hash. A block ends at 4,096 rows or 4 MiB uncompressed, whichever comes first.

Rows are globally sorted by `(bucket, dimension IDs)`. Within an uncompressed block:

- Integers use unsigned LEB128; signed values use ZigZag LEB128.
- Bucket values are delta-encoded from the previous row.
- Dimension dictionary IDs are encoded in declared order.
- `count` and `present_count` use `u64`.
- `sum`, `min`, and `max` use checked `i64` encodings.
- Histogram bins use `u64` and include underflow/overflow.
- Zstandard level 3 is the product default; benchmark ablations may use levels 1, 3, and 9.

The decoder validates every length and ID before allocation. It caps block size at 16 MiB, dimension count at eight for the physical format, and row count at the values declared in metadata. Corrupt or adversarial archives must fail without path traversal or uncontrolled allocation.

## 6. Public CLI and outputs

The binary is `tracefold`. Human-readable output goes to stderr for progress and stdout for requested data. Every command supports `--json` for a versioned machine-readable result.

```bash
# Generate deterministic canonical telemetry.
tracefold generate \
  --scenario standard --events 1000000 --seed 7 \
  --output data/synthetic/standard-1m-seed7.jsonl

# Normalize public or user data.
tracefold normalize \
  --adapter loghub-bgl --input data/raw/BGL.log \
  --output data/normalized/bgl.jsonl

# Encode an immutable archive.
tracefold encode \
  --input data/normalized/bgl.jsonl \
  --contract contracts/telemetry-v1.toml \
  --output artifacts/bgl.tfold

# Inspect storage composition and contract coverage.
tracefold inspect artifacts/bgl.tfold --json

# Execute a declared aggregate query.
tracefold query artifacts/bgl.tfold \
  --family event-volume \
  --start 2005-06-01T00:00:00Z \
  --end 2005-06-02T00:00:00Z \
  --where service=bgl \
  --group-by severity,event_type \
  --measure count --json

# Retrieve only data that the raw-retention contract preserves.
tracefold events artifacts/bgl.tfold --retained errors --jsonl

# Check format integrity or semantic correctness against source.
tracefold verify artifacts/bgl.tfold
tracefold verify artifacts/bgl.tfold \
  --source data/normalized/bgl.jsonl \
  --queries artifacts/queries/bgl-seed19.jsonl

# Fetch, run, and report benchmarks.
tracefold bench fetch --manifest benches/corpora.toml
tracefold bench smoke --output results/raw/smoke.jsonl
tracefold bench synthetic --output results/raw/synthetic.jsonl
tracefold bench public --output results/raw/public.jsonl
uv run --project scripts/report tracefold-report \
  --input results/raw --output results
```

Query JSON output contains `schema_version`, archive and contract hashes, family, normalized interval, filters, groupings, measures, exactness (`"exact"`), and sorted rows. Counts are JSON integers. Checked `i64` aggregate values are JSON integers.

Stable exit codes:

- `0`: success.
- `2`: CLI usage or contract validation error.
- `3`: malformed/invalid canonical input.
- `4`: corrupt or unsupported archive.
- `5`: query outside the preserved contract or raw data unavailable.
- `6`: semantic verification mismatch.
- `7`: benchmark/data acquisition failure.

## 7. Implementation architecture

Use a Rust workspace with four responsibilities:

- **Core:** canonical schema, contract parsing, oracle aggregation, retention classification, deterministic generator, and public-data adapters.
- **Archive:** dictionaries, aggregation/spill/merge, binary view codec, raw tiers, integrity verification, and query execution.
- **CLI:** stable commands, JSON output envelopes, progress, exit-code mapping, and safe filesystem behavior.
- **Benchmark:** corpus manifests, baseline adapters, process measurement, result rows, randomized experiment orchestration, and correctness gates.

Use `clap`, `serde`, `serde_json`, `toml`, `zstd`, `csv`, `rand_chacha`, `blake3`, `tempfile`, `anyhow`, and `thiserror`. Keep dependencies minimal. Rust tests use ordinary unit/integration tests plus `proptest` for codec and query invariants.

The Python `uv` reporting project uses DuckDB for raw and Parquet baselines, `zstandard` for stream baselines, and NumPy/Matplotlib for statistics and SVG generation. Pin all dependencies in `Cargo.lock` and `uv.lock`.

The Rust raw-data oracle and the DuckDB raw baseline must be independent implementations. A corpus is invalid if those two disagree on any generated benchmark query.

## 8. Data plan

### 8.1 Deterministic synthetic generator

Use ChaCha8 with the supplied `u64` seed. Distribution sampling must use fixed integer arithmetic rather than platform-dependent floating-point library behavior. Given identical version, parameters, and seed, generated canonical JSONL must be byte-identical across runs and supported hosts.

All primary scenarios cover 30 simulated days, generate monotonically increasing base timestamps, and then apply their declared out-of-order behavior. Trace lengths follow a capped geometric distribution. Bodies are JSON objects, not opaque binary.

Scenarios:

| Scenario | Parameters and purpose |
| --- | --- |
| `standard` | 16 services, 256 operations, Zipf 1.1 popularity, 1% errors, correlated status/severity, 64 message templates, median body about 160 bytes. |
| `low-cardinality` | 4 services, 16 operations, 0.1% errors, 20 templates, strong repetition; favorable to conventional compression. |
| `high-cardinality` | 64 services, up to 100,000 operations, 10,000 error codes, near-uniform dimension use; stresses view cardinality. |
| `high-entropy-body` | Standard dimensions with deterministic random 512-byte body fields; tests the value and danger of discarding undeclared payloads. |
| `error-burst` | 1% background errors plus twelve 30-minute intervals at 25% errors; stresses permanent raw error retention. |
| `out-of-order` | Standard distribution shuffled deterministically in 10,000-event windows; correctness-only scenario for batch timestamp handling. |

Dataset tiers:

- CI fixture: 10,000 events for every scenario.
- Main synthetic comparison: 1,000,000 events for the first five scenarios, seed 7.
- Scale sweep: `standard` at 100,000, 1,000,000, and 10,000,000 events.
- High-cardinality stress: attempt 10,000,000 events in every publish run; a timeout or resource failure remains a recorded result and does not abort later report generation.
- Sensitivity rerun: main 1,000,000-event scenarios with seed 19.

The generator also emits a metadata JSON containing parameters, seed, expected record count, timestamp bounds, field cardinalities, error count, and SHA-256/BLAKE3 of the JSONL.

### 8.2 Public corpora

Use the public [Loghub collection](https://github.com/logpai/loghub) and cite its associated paper in the generated report. The standard public download budget is under 1 GiB:

- **ZooKeeper:** approximately 74,380 lines and 9.95 MiB; use as the small public track.
- **BGL:** approximately 4,747,963 lines and 708.76 MiB; use as the main public scale track.

Loghub documents the datasets as freely accessible for research/academic use. Do not redistribute the raw corpora. The checked-in corpus manifest must record source URL, citation URL, expected compressed/raw bytes when published, and a SHA-256 lock. The fetch command downloads to `data/raw`, verifies the lock, and refuses unexpected content. Normalized output and downloaded data remain ignored by Git.

Adapters:

- `loghub-zookeeper` maps dataset name to `service`, component/logger to `operation` and `event_type`, level to normalized severity/status, parsed timestamp to `timestamp_ns`, and the entire original line to `body.raw_line`.
- `loghub-bgl` maps dataset name to `service`, component to operation/event type, node and facility fields to string attributes, alert labels to `ERROR`/`ERROR`, non-alert labels to `INFO`/`OK`, and the original line to `body.raw_line`.
- Dataset timezone assumptions and parse formats are recorded in normalization metadata.
- Malformed lines are not silently dropped. They become `UNKNOWN`/`UNSET` records with `event_type="unparsed"`, the original line in the body, and a parse-warning counter. A public normalization is invalid if more than 0.1% of lines are unparsed.

The repository may include the checked-in 2,000-line Loghub samples for tests only if their license/redistribution terms permit it; otherwise create structurally representative synthetic fixtures and fetch samples during explicit benchmark setup.

### 8.3 Optional external validation

OpenTelemetry's official demo and `telemetrygen` are suitable future sources, but v1 completion does not depend on Docker Compose or a live collector. The canonical schema should remain easy to map from OTLP JSON later.

## 9. Baselines

Every storage baseline receives the same normalized canonical JSONL. Normalization time is reported separately and excluded from encoding comparisons.

### Raw-preserving baselines

- Canonical JSONL, uncompressed.
- JSONL + gzip level 6.
- JSONL + Zstandard levels 3 and 9.
- Raw Parquet with Zstandard through DuckDB, storing fixed columns plus canonical JSON strings for attributes/body.
- DuckDB raw table with default compression and no materialized aggregate views.

For JSONL/gzip/Zstandard query timing, stream-decompress and scan with the Rust raw oracle for every process-cold batch and keep the decoded stream available only within a warm batch. Raw Parquet and DuckDB execute equivalent SQL through DuckDB. “Raw-preserving” means preserving canonical event values, including attributes and body, not preserving JSON whitespace or key order.

### Semantic baseline

- `views-parquet-zstd` computes the **identical** aggregate cells, dictionaries, hot raw tier, and permanent error tier required by the TraceFold contract, but stores views as Parquet/Zstandard and queries them through DuckDB.
- It is the primary baseline for deciding whether the custom `.tfv` encoding adds value beyond changing semantics.
- Its archive byte count includes view files, raw tiers, contract, metadata, and indexes, exactly as TraceFold's count does.

### TraceFold variants

- `tracefold-separate-zstd3`: primary product configuration.
- `tracefold-separate-zstd1` and `tracefold-separate-zstd9`: codec-level ablations.
- `tracefold-unified-zstd3`: dimension-layout ablation when legal.

### Non-comparable lower bound

An `answer-sheet` experiment may store results for the finite generated query instances. It must be labeled non-comparable because it does not preserve the runtime query family and is generated only after queries exist. It provides a descriptive lower bound and never appears in primary winner rankings.

## 10. Benchmark anti-cheating and correctness rules

The encoder receives only canonical data and the family manifest. It must never receive benchmark query instances.

For every dataset/baseline/contract combination:

1. Normalize or generate canonical JSONL and freeze its hashes.
2. Encode the archive and freeze all archive hashes.
3. Only then run a separate query-workload generator with seed 19.
4. Generate runtime intervals, filters, groupings, and measure subsets within declared families.
5. Execute each instance against the Rust raw oracle, DuckDB raw baseline, semantic Parquet baseline, and TraceFold.
6. Canonicalize and compare results byte-for-byte after stable sorting.
7. Mark the entire benchmark row invalid on any mismatch; never include invalid timings in aggregate performance charts.

The query generator may scan raw data after encoding to discover legal values, but its output timestamp and hash must postdate and differ from the archive record. The benchmark result records both hashes and the execution order.

Generate 40 query instances for every applicable family:

- Ten unfiltered queries.
- Ten with one equality filter.
- Ten with two equality filters.
- Ten with an `IN` filter.

Choose groupings from legal subsets and time windows from one hour, six hours, 24 hours, seven days, and the full aligned span. If a family has no non-null measure values in a public corpus, record it as inapplicable rather than inventing values.

Also generate 20 deliberately illegal queries per archive. Every implementation must reject them with the expected contract error.

## 11. Benchmark measurements and protocol

### Metrics

- Apparent archive bytes and allocated disk bytes, with apparent bytes primary.
- Compression ratio relative to canonical JSONL.
- Bytes per source event.
- Component breakdown: views, dictionaries, hot raw, error raw, indexes/metadata.
- Raw recoverability: retained events divided by total events.
- Encode wall time, CPU time when available, throughput in source MiB/s and events/s.
- Peak RSS and temporary-disk high-water mark.
- Spill count and spill bytes.
- Query batch wall time and per-query p50, p90, p95, p99.
- Bytes read/decompressed per query when the implementation can measure it.
- Integrity and semantic mismatch counts.
- Contract coverage and explicit rejection counts.

### Timing protocol

- Build Rust with `cargo build --release --locked`.
- Run baselines as child processes so wall time, exit status, stdout hash, stderr, and peak RSS use the same harness.
- On Linux/WSL, sample `/proc/<pid>/status` for peak RSS; on unsupported hosts, emit null rather than estimate.
- Randomize baseline order deterministically per dataset/trial.
- Run one untimed correctness pass before timing.
- Use five encode trials for small/medium inputs and three for BGL and 10-million-event inputs.
- Warm-query mode opens the archive once, performs two unreported warmup loops, then ten measured shuffled loops over the workload.
- Process-cold mode starts a new process for the full query batch five times. Do not claim OS page-cache coldness and do not require privileged cache dropping.
- Record normalization/fetch/generation time separately from encode time.
- Record failed and timed-out rows with structured failure kinds.
- Default timeout: 30 minutes per encode trial and 10 minutes per query batch.
- Record CPU model, logical cores, total memory, OS/kernel, filesystem type, WSL/container/native classification, tool versions, Git commit, contract hash, corpus hash, and benchmark command.

### Statistical reporting

- Report medians as primary timing estimates, plus p90/p95 where relevant.
- Report all individual trials in raw JSONL.
- Compute deterministic 95% bootstrap confidence intervals for median ratios using 10,000 resamples and seed 23.
- Do not use significance stars or overstate differences smaller than run-to-run variation.
- Never average compression ratios across corpora without also showing each corpus.

## 12. Experiment matrix

Keep the matrix staged so the full run remains tractable.

### E0 — Correctness and determinism

- Every 10,000-event scenario.
- Default contract, one-minute bucket, 24-hour retention.
- All legal query shapes, illegal queries, corruption cases, forced spill, out-of-order input, nulls, and overflow.
- Encode the same fixture twice and require identical recursive archive hashes.

### E1 — Primary comparison

- Synthetic first five scenarios at one million events, seed 7.
- Public ZooKeeper and BGL.
- Default one-minute bucket and 24-hour retention.
- All raw-preserving, semantic, and primary TraceFold baselines.

### E2 — Retention frontier

- `standard` one-million-event synthetic and BGL.
- Recent windows: `0`, one hour, six hours, 24 hours, seven days, and `all`.
- Default one-minute bucket.
- Compare Zstandard raw, Parquet raw, semantic Parquet, and TraceFold primary.

### E3 — Time-precision frontier

- `standard` one-million-event synthetic and BGL.
- Buckets: one second, one minute, five minutes, and one hour.
- Default 24-hour retention.
- Report archive size and query latency alongside the explicit time precision; never call coarser bucketing an exact replacement for finer semantics.

### E4 — Scale and cardinality

- `standard` at 100,000, one million, and ten million events.
- `high-cardinality` at one million and ten million events; the ten-million attempt is mandatory, while a structured timeout/resource failure is an acceptable measured outcome.
- Report scaling, cell counts, spill behavior, memory, and whether views exceed raw baselines.

### E5 — Error-retention cost

- Derive deterministic standard variants with 0%, 0.1%, 1%, 5%, and 20% errors.
- Hold all other distributions and the 24-hour window constant.
- Plot error rate against storage and raw recoverability.

### E6 — Format/layout ablation

- Standard, high-cardinality, and BGL.
- Compare semantic Parquet, separate views at Zstandard 1/3/9, and unified layout where legal.
- Attribute any benefit to semantics versus custom encoding separately.

### E7 — Seed sensitivity

- Repeat the one-million-event primary synthetic matrix with seed 19.
- Report whether conclusions change direction, not merely whether exact numbers differ.

## 13. Benchmark result schema and reporting

Every benchmark attempt emits one JSONL row with `schema_version: 1` containing:

- Run ID, timestamp, Git commit, host metadata, command, and tool versions.
- Dataset identity, source/generator version, seed, parameters, hashes, record count, span, and normalized bytes.
- Contract and archive hashes, bucket, retention, layout, baseline, and codec settings.
- Trial number, randomized order, timing mode, query-workload hash, and query count.
- Storage, encoding, memory, spill, query, correctness, and raw-recoverability metrics.
- `success`, structured `failure_kind`, and error text.

The report command accepts one file or a directory, validates schemas, copies immutable raw inputs into `results/site-data/raw`, and creates:

```text
results/
  summary.md
  paper.md
  site-data/
    summary.json
    methodology.json
    tables/*.json
    charts/*.svg
    raw/*.jsonl
```

Required charts:

- Archive bytes and compression ratio by corpus/baseline.
- Storage-versus-query-latency Pareto scatter.
- Retention-window frontier with raw recoverability.
- Bucket-width frontier labeled with time precision.
- Encode throughput and peak RSS.
- Archive component stacked bars.
- Cardinality and scale curves.
- Error-rate retention cost.
- Semantic Parquet versus custom format ablation.
- Failure/mismatch summary; empty only when genuinely zero.

Charts must show regressions and avoid truncating axes deceptively. Generated JSON must be compact and stable enough for a later Vercel/Next.js paper page without rerunning benchmarks.

## 14. Paper requirements

Produce a research-style technical report, not a claim of peer-reviewed novelty. `results/paper.md` is assembled from a checked-in narrative template plus generated tables/charts.

Required sections:

1. Abstract.
2. Motivation and explicit semantic tradeoff.
3. Query-family and raw-retention contract.
4. System architecture and archive format.
5. Research questions and hypotheses.
6. Dataset and workload methodology.
7. Baselines and fairness rules.
8. Results, including negative findings.
9. Threats to validity.
10. Limitations and unsupported queries.
11. Related work: lossless compression, columnar formats, materialized views/data cubes, tiered telemetry storage, and query-aware compression.
12. Reproducibility instructions.
13. Conclusion stating only claims supported by generated results.

Threats to discuss explicitly:

- Synthetic distributions may favor declared dimensions or payload dropping.
- Loghub logs are not full modern distributed traces.
- The declared query set is selected by the authors.
- Time bucketing changes semantics.
- Permanent raw error retention depends on normalization/classification quality.
- DuckDB/Parquet and TraceFold use different query engines.
- Process-cold results are not guaranteed disk-cold.
- One benchmark host does not establish universal performance.
- Materialized views, not a novel entropy coder, may explain most gains.

Use primary sources where possible. At minimum, cite Loghub, Zstandard, Apache Parquet, DuckDB, OpenTelemetry's data model/semantic conventions, and foundational materialized-view or data-cube work. Keep a checked-in bibliography with stable URLs/DOIs.

## 15. Implementation sequence

Implement in this order; each milestone must leave tests green.

### Milestone 1 — Scaffold and contracts

- Create the Rust workspace, Python reporting project, locks, CI, license, README, and default manifest.
- Implement canonical schema validation, duration parsing, contract validation, stable JSON output envelopes, and exit codes.
- Implement the independent raw oracle and golden query-result fixtures.

### Milestone 2 — Data generation and normalization

- Implement deterministic synthetic scenarios and metadata.
- Implement ZooKeeper/BGL adapters and corpus fetch/lock behavior.
- Add byte-determinism, malformed-line, timestamp, and mapping tests.

### Milestone 3 — Archive writer and verifier

- Implement pass-one validation/dictionaries, pass-two retention and aggregation, spill runs, merge, raw tiers, metadata, checksums, and atomic finalization.
- Implement separate layout first, then unified ablation.
- Add deterministic archive, forced-spill, corruption, cleanup, and large-cardinality tests.

### Milestone 4 — Query and retained-event readers

- Implement block pruning, decode, filters, regrouping, measure selection, raw retained scans, inspect, and verify.
- Enforce every contract boundary and stable ordering.
- Property-test archive answers against the raw oracle across generated small datasets and random legal queries.

### Milestone 5 — Baselines and harness

- Implement gzip/Zstandard, DuckDB raw, raw Parquet, and semantic Parquet baselines.
- Implement process timing, RSS sampling, randomized order, timeouts, result schema, failure retention, and three-way correctness checks.
- Implement post-encode query generation so the anti-cheating order is mechanically enforced.

### Milestone 6 — Reporting and paper

- Implement schema validation, statistics, bootstrap intervals, Markdown tables, SVG charts, site JSON, and paper assembly.
- Add fixture result rows and snapshot/semantic tests for every report output.
- Make missing or failed data explicit rather than silently suppressing charts.

### Milestone 7 — Full evaluation and hardening

- Run E0 and fix correctness before measuring performance.
- Run smoke, main synthetic, public, and staged ablations.
- Review benchmark commands and raw rows for fairness.
- Generate the final paper and ensure every quantitative sentence is traceable to a table or chart.
- Run the complete quality gate and document known regressions.

## 16. Test plan

### Rust unit/property tests

- Canonical schema validation, nulls, unknown fields, duplicate IDs, timestamp extremes, and integer overflow.
- Contract parsing, invalid dimensions/operations, histogram boundaries, bucket alignment, and duration parsing.
- Dictionary determinism and null ID behavior.
- Varint/ZigZag round trips, block boundaries, hash failures, invalid lengths, invalid IDs, and truncation.
- Aggregate associativity across spills and merge order.
- Retention cutoff boundaries and error classification.
- Query filters, grouping subsets, histograms, empty results, stable ordering, and illegal-query rejection.
- Generator determinism and scenario invariants.

### Integration tests

- Encode/query/inspect/events/verify end to end on tiny fixtures.
- Archive hashes identical across repeated encodes.
- Forced spill output equals no-spill output.
- Recent errors are stored once yet appear correctly in aggregate and raw results.
- Interrupted/failed encodes leave no published partial archive.
- Tampering with every archive component is detected.
- Raw oracle, DuckDB raw, semantic Parquet, and TraceFold agree exactly.
- Query workload timestamps prove generation after archive finalization.
- Public-adapter sample mappings and unparsed thresholds.
- Benchmark failures remain in JSONL and reports.

### Python/report tests

- Result-schema rejection and mixed-version handling.
- Median, percentile, ratio, and deterministic bootstrap fixtures.
- Failure rows excluded from performance aggregates but included in failure tables.
- Stable summary/site JSON and valid standalone SVG.
- Negative/regression charts render correctly.
- Paper assembly never leaves placeholders or claims without data.

### Quality gate

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
uv sync --project scripts/report --locked
uv run --project scripts/report python -m unittest discover -s scripts/report/tests
cargo run --release -p tracefold-cli -- bench smoke --output /tmp/tracefold-smoke.jsonl
uv run --project scripts/report tracefold-report --input /tmp/tracefold-smoke.jsonl --output /tmp/tracefold-report
```

## 17. Acceptance criteria

V1 is ready when all of the following are true:

- A fresh checkout can build and run the smoke benchmark using documented commands.
- Synthetic data is byte-deterministic from recorded seeds.
- ZooKeeper and BGL acquisition is pinned, verified, normalized, and cited.
- The default archive implements 24-hour exact recent retention and indefinite exact error retention without duplication.
- All declared aggregate queries match both independent raw oracles exactly.
- All illegal queries fail with exit code 5 and a machine-readable explanation.
- Archive corruption is detected; encoding is atomic; forced spills preserve answers.
- The ten-million-event standard track completes within 2 GiB peak RSS using spills on the current 31-GiB development host, or a failure row and documented investigation demonstrate why the bound could not be met.
- Primary benchmarks include every required raw-preserving and semantic baseline.
- Query instances are generated after archives and their hashes/order are recorded.
- Raw JSONL benchmark rows, generated charts/tables, summary, and paper can be regenerated with one documented command sequence.
- The paper clearly distinguishes semantic savings from custom-format savings and includes unfavorable results.
- `results/site-data` is sufficient for a later hosted paper page.
- No large corpus, generated archive, secret, or machine-specific absolute path is committed.

## 18. Privacy, safety, and compatibility

- Telemetry bodies and attributes may contain secrets. TraceFold does not redact them.
- Exact recent and error tiers should be treated as sensitive. Encoding prints this warning unless `--quiet` is set.
- Dropping old bodies is not secure deletion, encryption, or proof that all sensitive values disappeared; declared dimensions and dictionaries remain.
- Normalization and archive operations are local. Dataset fetch is the only required networked command.
- The v1 runtime target is Linux, including WSL2. macOS should compile where dependencies permit, but publish measurements are host-specific.
- Archive, contract, canonical event, query-result, and benchmark-result schemas are independently versioned at `1`.
- Before version 0.2, incompatible format changes may reject old archives but must fail clearly. Do not add migration machinery to v1.

## 19. Known risks and mitigations

- **Degenerate materialized-answer benchmark:** Prevented by generating runtime query instances only after archive finalization and preserving parameterized families rather than finite answers.
- **Unfair raw baseline:** Include semantic Parquet with identical views and retention as the primary format baseline.
- **Cardinality explosion:** Measure it, spill to disk, cap dimensions, and report cases where query preservation loses.
- **Synthetic overfitting:** Use fixed public corpora, a second seed, adversarial scenarios, and per-corpus results.
- **Parser misclassification:** Preserve raw lines in canonical bodies, record parse warnings, enforce a 0.1% threshold, and discuss error-label dependence.
- **Benchmark noise:** Repeat trials, randomize order, preserve raw rows, use confidence intervals, and avoid disk-cold claims.
- **Paper-first scope creep:** No streaming, SQL parser, approximate sketches, OTLP service, or web UI in v1.

## 20. Post-v1 possibilities

These are explicitly deferred:

- Continuous ingestion with background hot-to-cold compaction.
- Error-bounded quantiles, distinct counts, and sketches.
- Learned contracts from historical query logs.
- A greedy cuboid planner beyond separate/unified layouts.
- Direct OTLP JSON/protobuf and OpenTelemetry Collector integration.
- Indexed raw-event retrieval and selective permanent retention predicates.
- Encryption and redaction.
- A Vercel-hosted interactive paper using `results/site-data`.

The v1 paper should end by identifying which of these is justified by measured results rather than assuming that every extension is worthwhile.

## Appendix A — Copy/paste implementation prompt

```text
Read /home/user/tracefold/PRD.md completely and implement TraceFold v1 end to end in /home/user/tracefold. Treat the PRD as decision complete and preserve its semantic, fairness, data, benchmark, and paper requirements. Continue through all milestones without stopping after scaffolding or a partial prototype. Run correctness gates before performance experiments; preserve negative and failed benchmark rows; do not fabricate favorable results. Do not inspect indexdataset. Do not push or deploy. When finished, report implemented behavior, benchmark evidence actually obtained, quality-gate results, known limitations, and exact commands for reproducing the paper.
```
