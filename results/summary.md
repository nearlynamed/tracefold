# TraceFold benchmark summary

**Status:** Technical report — not peer reviewed  
**By:** nearlynamed  
**Attempts:** 8 (8 successful, 0 failed)

| Dataset | Baseline | Archive bytes (median) | Compression ratio | Encode time (ns) | Query batch (ns) |
| --- | --- | ---: | ---: | ---: | ---: |
| smoke-standard-10k | duckdb-raw | 798720.0 | 5.806281300080128 | 136111647.0 | 1511619571.0 |
| smoke-standard-10k | gzip-6 | 459351.0 | 10.095968007036014 | 42506278.0 | None |
| smoke-standard-10k | jsonl | 4637593.0 | 1.0 | 300.0 | 604873594.0 |
| smoke-standard-10k | parquet-raw-zstd | 250620.0 | 18.50448088739925 | 12343549.0 | 1519463749.0 |
| smoke-standard-10k | tracefold-separate-zstd3 | 378080.0 | 12.266168535759627 | 137826498.0 | 599260152.0 |
| smoke-standard-10k | views-parquet-zstd | 1377211.0 | 3.3673801617907495 | 78589420.0 | 1671545646.0 |
| smoke-standard-10k | zstd-3 | 456689.0 | 10.154816516272561 | 13688734.0 | None |
| smoke-standard-10k | zstd-9 | 390177.0 | 11.885869746294631 | 59699994.0 | None |

## Failures

No failures were recorded in this result set.
