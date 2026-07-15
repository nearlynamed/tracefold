# TraceFold benchmark summary

**Status:** Technical report — not peer reviewed  
**By:** nearlynamed  
**Attempts:** 24 (24 successful, 0 failed)

| Dataset | Baseline | Archive bytes (median) | Compression ratio | Encode time (ns) | Query batch (ns) |
| --- | --- | ---: | ---: | ---: | ---: |
| loghub-bgl | duckdb-raw | 341585920.0 | 5.86702917087449 | 8935793158.0 | 8652737356.0 |
| loghub-bgl | gzip-6 | 95423564.0 | 21.00209291071962 | 7629044049.0 | 9644742135.0 |
| loghub-bgl | jsonl | 2004094557.0 | 1.0 | 600.0 | 9153897897.0 |
| loghub-bgl | parquet-raw-zstd | 80924435.0 | 24.765011420839702 | 2947812349.0 | 9166627969.0 |
| loghub-bgl | tracefold-separate-zstd3 | 6776559.0 | 295.73926191744215 | 24849741253.0 | 1906511407.0 |
| loghub-bgl | views-parquet-zstd | 6422450.0 | 312.0451785533558 | 714465605.0 | 10145771216.0 |
| loghub-bgl | zstd-3 | 100358115.0 | 19.969432038455484 | 2395035128.0 | 9630280141.0 |
| loghub-bgl | zstd-9 | 73285895.0 | 27.346252058462273 | 14547923847.0 | 9426134639.0 |
| loghub-zookeeper | duckdb-raw | 5779456.0 | 5.149039286742559 | 419258750.0 | 2881090974.0 |
| loghub-zookeeper | gzip-6 | 939710.0 | 31.667903927807515 | 87977904.0 | 417127221.0 |
| loghub-zookeeper | jsonl | 29758646.0 | 1.0 | 1200.0 | 419853979.0 |
| loghub-zookeeper | parquet-raw-zstd | 642771.0 | 46.297430966860674 | 65006687.0 | 2981474874.0 |
| loghub-zookeeper | tracefold-separate-zstd3 | 105615.0 | 281.76533636320596 | 364976297.0 | 601970151.0 |
| loghub-zookeeper | views-parquet-zstd | 211129.0 | 140.95006370512814 | 94024507.0 | 3176942288.0 |
| loghub-zookeeper | zstd-3 | 978507.0 | 30.41229751039083 | 36441688.0 | 423799663.0 |
| loghub-zookeeper | zstd-9 | 693507.0 | 42.91037581451953 | 190692826.0 | 416360343.0 |
| smoke-standard-10000 | duckdb-raw | 798720.0 | 5.806281300080128 | 144796639.0 | 1476537558.0 |
| smoke-standard-10000 | gzip-6 | 459351.0 | 10.095968007036014 | 40404867.0 | 200230057.0 |
| smoke-standard-10000 | jsonl | 4637593.0 | 1.0 | 400.0 | 206561213.0 |
| smoke-standard-10000 | parquet-raw-zstd | 250620.0 | 18.50448088739925 | 13190713.0 | 1555421818.0 |
| smoke-standard-10000 | tracefold-separate-zstd3 | 378096.0 | 12.265649464686216 | 142952868.0 | 586683934.0 |
| smoke-standard-10000 | views-parquet-zstd | 1377305.0 | 3.3671503407015875 | 77566142.0 | 1727934370.0 |
| smoke-standard-10000 | zstd-3 | 456689.0 | 10.154816516272561 | 14783417.0 | 198737321.0 |
| smoke-standard-10000 | zstd-9 | 390177.0 | 11.885869746294631 | 59390075.0 | 203377630.0 |

## Failures

No failures were recorded in this result set.
