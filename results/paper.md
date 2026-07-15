# TraceFold: Query-Preserving Compression for Tiered Telemetry Archives

**nearlynamed**  
**Technical report — not peer reviewed**

## Abstract

TraceFold evaluates whether a telemetry archive can reduce storage and query cost by preserving declared query families instead of reconstructing every historical event. This report is generated from immutable benchmark rows and intentionally preserves negative results.

## Motivation and semantic tradeoff

Recent records and all errors remain exactly recoverable. Older successful payloads are discarded after contributing to exact bucketed aggregate views. Arbitrary historical raw retrieval and undeclared queries are explicitly unavailable.

## System architecture

The Rust encoder validates canonical JSONL twice, builds deterministic dictionaries, retains raw tiers, spills sorted aggregate runs under memory pressure, and writes independently checksummed Zstandard blocks. The reader prunes blocks by time and regroups only within a declared family.

## Research questions and hypotheses

The artifact measures storage, query latency, semantic knobs, workload shape, custom encoding benefit, and ingestion cost. Favorable results are hypotheses, not completion criteria.

## Dataset and workload methodology

All source artifacts are capped at 1,073,741,824 bytes under the declared downloaded/generated-source basis. The largest observed download/generated source was 57,489,019 bytes; its canonical normalization expanded to 2,004,094,557 bytes. Synthetic data is seeded; public data comes from Loghub. Each archive is finalized before its 200 legal and 20 illegal queries are generated.

## Results

The current publication contains 24 baseline attempts: 24 successful and 0 failed, with 0 semantic mismatches. Table 1 is generated directly from the immutable raw result rows.

| Dataset | Baseline | Archive bytes | Compression | Encode (ms) | Query batch (ms) |
| --- | --- | ---: | ---: | ---: | ---: |
| loghub-bgl | duckdb-raw | 341,585,920 | 5.87× | 8935.79 | 8652.74 |
| loghub-bgl | gzip-6 | 95,423,564 | 21.00× | 7629.04 | 9644.74 |
| loghub-bgl | jsonl | 2,004,094,557 | 1.00× | 0.00 | 9153.90 |
| loghub-bgl | parquet-raw-zstd | 80,924,435 | 24.77× | 2947.81 | 9166.63 |
| loghub-bgl | tracefold-separate-zstd3 | 6,776,559 | 295.74× | 24849.74 | 1906.51 |
| loghub-bgl | views-parquet-zstd | 6,422,450 | 312.05× | 714.47 | 10145.77 |
| loghub-bgl | zstd-3 | 100,358,115 | 19.97× | 2395.04 | 9630.28 |
| loghub-bgl | zstd-9 | 73,285,895 | 27.35× | 14547.92 | 9426.13 |
| loghub-zookeeper | duckdb-raw | 5,779,456 | 5.15× | 419.26 | 2881.09 |
| loghub-zookeeper | gzip-6 | 939,710 | 31.67× | 87.98 | 417.13 |
| loghub-zookeeper | jsonl | 29,758,646 | 1.00× | 0.00 | 419.85 |
| loghub-zookeeper | parquet-raw-zstd | 642,771 | 46.30× | 65.01 | 2981.47 |
| loghub-zookeeper | tracefold-separate-zstd3 | 105,615 | 281.77× | 364.98 | 601.97 |
| loghub-zookeeper | views-parquet-zstd | 211,129 | 140.95× | 94.02 | 3176.94 |
| loghub-zookeeper | zstd-3 | 978,507 | 30.41× | 36.44 | 423.80 |
| loghub-zookeeper | zstd-9 | 693,507 | 42.91× | 190.69 | 416.36 |
| smoke-standard-10000 | duckdb-raw | 798,720 | 5.81× | 144.80 | 1476.54 |
| smoke-standard-10000 | gzip-6 | 459,351 | 10.10× | 40.40 | 200.23 |
| smoke-standard-10000 | jsonl | 4,637,593 | 1.00× | 0.00 | 206.56 |
| smoke-standard-10000 | parquet-raw-zstd | 250,620 | 18.50× | 13.19 | 1555.42 |
| smoke-standard-10000 | tracefold-separate-zstd3 | 378,096 | 12.27× | 142.95 | 586.68 |
| smoke-standard-10000 | views-parquet-zstd | 1,377,305 | 3.37× | 77.57 | 1727.93 |
| smoke-standard-10000 | zstd-3 | 456,689 | 10.15× | 14.78 | 198.74 |
| smoke-standard-10000 | zstd-9 | 390,177 | 11.89× | 59.39 | 203.38 |

- On `loghub-bgl`, TraceFold was 5.5% larger than the view-equivalent semantic Parquet archive.
- Relative to raw Parquet on `loghub-bgl`, TraceFold was 91.6% smaller; this comparison combines the semantic tradeoff with the storage format.
- On `loghub-zookeeper`, TraceFold was 50.0% smaller than the view-equivalent semantic Parquet archive.
- Relative to raw Parquet on `loghub-zookeeper`, TraceFold was 83.6% smaller; this comparison combines the semantic tradeoff with the storage format.
- On `smoke-standard-10000`, TraceFold was 72.5% smaller than the view-equivalent semantic Parquet archive.
- Relative to raw Parquet on `smoke-standard-10000`, TraceFold was 50.9% larger; this comparison combines the semantic tradeoff with the storage format.

## Threats to validity

Synthetic distributions, author-selected query families, Loghub's age and shape, bucket semantics, normalization quality, different query engines, page-cache effects, and a single WSL2 host all limit generalization. Materialized views may explain more benefit than the custom codec. The checked-in snapshot contains one measured attempt per corpus/baseline, so its timing values characterize this run rather than a stable population estimate.

## Limitations

TraceFold v1 does not support arbitrary SQL, joins, regex predicates, quantiles, distinct counts, streaming ingestion, or reconstruction of old successful payloads. Peak RSS is not measured in this snapshot, and the retention, bucket-width, scale/cardinality, error-rate, format-level, and seed-sensitivity frontiers specified as E2–E7 remain future benchmark stages. The report therefore makes no claim about the 10-million-event memory target.

## Related work

The checked-in bibliography covers Zstandard, Apache Parquet, DuckDB, OpenTelemetry, Loghub, materialized views, and data cubes.

## Reproducibility

Use the locked Rust, Python, and Node dependencies and the commands documented in the repository. Raw public corpora are fetched from their original source and are not redistributed.

## Conclusion

Conclusions must be limited to the generated measurements. Failures and regressions remain part of the artifact rather than being filtered from publication.
