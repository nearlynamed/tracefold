# TraceFold

TraceFold is an executable research artifact for query-preserving compression of tiered telemetry archives. It keeps exact recent records and every error record while replacing older successful payloads with exact, bucketed aggregate views for a declared query contract.

The project includes a Rust archive format and CLI, deterministic synthetic and Loghub corpus adapters, independent DuckDB/Parquet baselines, a reproducible Python reporting pipeline, and a static interactive paper. The full design and fairness rules are specified in [PRD.md](PRD.md).

> **Research status:** technical report, not peer reviewed. TraceFold preserves only declared query families. It cannot reconstruct old successful payloads or answer arbitrary SQL, joins, regex filters, quantiles, or distinct counts.

## Quick verification

Required tools are Rust 1.96, Python 3.11+ with `uv`, Node 24, and pnpm 11.11.

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
uv sync --project scripts/report --locked
uv run --project scripts/report python -m unittest discover -s scripts/report/tests
pnpm install --frozen-lockfile
pnpm --dir site check
pnpm --dir site test
pnpm --dir site build
```

Run the complete 10,000-event smoke publication, including all eight storage baselines and 200 legal plus 20 illegal queries:

```bash
scripts/reproduce-smoke.sh
```

Generated corpora, archives, and baseline databases stay under ignored `data/` and `artifacts/` paths. Curated machine-readable result rows, charts, tables, and the paper are written to `results/`.

## CLI

Generate canonical telemetry and encode an archive:

```bash
tracefold generate \
  --scenario standard --events 100000 --seed 7 \
  --output data/generated/standard.jsonl

tracefold encode \
  --input data/generated/standard.jsonl \
  --contract contracts/telemetry-v1.toml \
  --output artifacts/standard.tfold
```

Inspect, query, recover retained events, and verify integrity:

```bash
tracefold inspect artifacts/standard.tfold
tracefold query artifacts/standard.tfold \
  --family event-volume \
  --start 1784064000000000000 --end 1784150400000000000 \
  --group-by service,status --measure count
tracefold events artifacts/standard.tfold --retained errors --jsonl
tracefold verify artifacts/standard.tfold
```

Commands use stable JSON schemas. Exit codes distinguish canonical-input (`3`), archive-integrity (`4`), contract/query (`5`), semantic-mismatch (`6`), and benchmark/acquisition (`7`) failures. Encoding warns because retained bodies and attributes may contain secrets; TraceFold does not redact or encrypt them.

## Public corpora and the 1 GiB cap

The public manifest pins exact Zenodo Loghub artifacts, byte lengths, inner paths, and SHA-256 digests. The cap applies to the downloaded source artifact; extracted and normalized intermediates may be larger.

```bash
target/release/tracefold bench fetch \
  --manifest benches/corpora.toml \
  --max-source-bytes 1073741824

target/release/tracefold normalize \
  --adapter loghub-zookeeper \
  --input data/raw/loghub-zookeeper.log \
  --output data/normalized/zookeeper.jsonl

target/release/tracefold normalize \
  --adapter loghub-bgl \
  --input data/raw/loghub-bgl.log \
  --output data/normalized/bgl.jsonl

target/release/tracefold bench public \
  --output results/raw-work/public.jsonl \
  --max-source-bytes 1073741824
```

Malformed public lines remain as `unparsed` canonical records. Normalization fails if the warning rate exceeds 0.1%.

## Benchmark protocol

Every benchmark freezes canonical input and archive hashes before generating a seeded query workload. The default contract produces 40 legal queries for each of five families and 20 illegal queries. TraceFold answers are compared byte-for-byte with the Rust raw oracle; DuckDB raw, raw Parquet, and view-equivalent Parquet are independently checked against hashed oracle rows.

Included storage baselines are canonical JSONL, gzip-6, Zstandard 3/9, DuckDB raw, Parquet/Zstandard raw, semantic Parquet/Zstandard, and TraceFold separate-view/Zstandard-3. Failures and semantic mismatches remain in raw result rows and are excluded only from performance aggregates.

The checked-in publication is intentionally host-specific. It does not claim disk-cold cache state or universal performance. See the generated paper for threats to validity and [results/summary.md](results/summary.md) for the measured snapshot.

## Research site

The Next.js App Router site is a static export. It reads only generated files from `results/site-data`, verifies publication metadata and raw-artifact hashes during its sync step, and uses no analytics or runtime backend.

```bash
pnpm --dir site dev
pnpm --dir site build
```

Production is published at [tracefold.vercel.app](https://tracefold.vercel.app), with source at [github.com/nearlynamed/tracefold](https://github.com/nearlynamed/tracefold).

## License

Licensed under either Apache-2.0 or MIT, at your option.
