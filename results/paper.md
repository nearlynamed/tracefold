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

All source artifacts are capped at 1073741824 bytes under the declared downloaded/generated-source basis. Synthetic data is seeded; public data comes from Loghub. Benchmark queries are created after archive finalization.

## Results

The current publication contains 8 attempts: 8 successful and 0 failed. See `summary.md` and the generated figures for the complete per-corpus results.

## Threats to validity

Synthetic distributions, author-selected query families, Loghub's age and shape, bucket semantics, normalization quality, different query engines, page-cache effects, and a single WSL2 host all limit generalization. Materialized views may explain more benefit than the custom codec.

## Limitations

TraceFold v1 does not support arbitrary SQL, joins, regex predicates, quantiles, distinct counts, streaming ingestion, or reconstruction of old successful payloads.

## Related work

The checked-in bibliography covers Zstandard, Apache Parquet, DuckDB, OpenTelemetry, Loghub, materialized views, and data cubes.

## Reproducibility

Use the locked Rust, Python, and Node dependencies and the commands documented in the repository. Raw public corpora are fetched from their original source and are not redistributed.

## Conclusion

Conclusions must be limited to the generated measurements. Failures and regressions remain part of the artifact rather than being filtered from publication.
