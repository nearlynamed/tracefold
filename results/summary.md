# TraceFold benchmark summary

**Status:** Technical report — not peer reviewed
**By:** nearlynamed
**Snapshot:** `6666bcc18a5b6627da0824ac3e5536aa95287e538653f16ea89b7973e17dc937`
**Attempts:** 39 (39 successful, 0 failed)

| Dataset | Baseline | Archive bytes (median) | Compression ratio | Encode time (ns) | Query batch (ns) |
| --- | --- | ---: | ---: | ---: | ---: |
| loghub-bgl | duckdb-raw | 342896640.0 | 5.844602493042801 | 8873726551.0 | 8755092617.0 |
| loghub-bgl | gzip-6 | 95423564.0 | 21.00209291071962 | 7701684685.0 | 9431680956.0 |
| loghub-bgl | jsonl | 2004094557.0 | 1.0 | 800.0 | 9074821672.0 |
| loghub-bgl | parquet-raw-zstd | 80953926.0 | 24.75598968479923 | 2171080882.0 | 8992310769.0 |
| loghub-bgl | tracefold-auto-zstd9 | 4698677.0 | 426.52315896581104 | 47051453119.0 | 2593739486.0 |
| loghub-bgl | views-parquet-zstd | 6426634.0 | 311.8420244563484 | 651705719.0 | 10041476198.0 |
| loghub-bgl | zstd-3 | 100358115.0 | 19.969432038455484 | 2587710799.0 | 9384315504.0 |
| loghub-bgl | zstd-9 | 73285895.0 | 27.346252058462273 | 14023699355.0 | 9541499576.0 |
| loghub-zookeeper | duckdb-raw | 5517312.0 | 5.3936855483249815 | 339171401.0 | 2965184915.0 |
| loghub-zookeeper | gzip-6 | 939710.0 | 31.667903927807515 | 82110990.0 | 422073881.0 |
| loghub-zookeeper | jsonl | 29758646.0 | 1.0 | 1000.0 | 416824375.0 |
| loghub-zookeeper | parquet-raw-zstd | 642771.0 | 46.297430966860674 | 64419639.0 | 3121737848.0 |
| loghub-zookeeper | tracefold-auto-zstd9 | 53964.0 | 551.4536728189163 | 727508962.0 | 841489106.0 |
| loghub-zookeeper | views-parquet-zstd | 210671.0 | 141.25648997726313 | 106770189.0 | 3292329435.0 |
| loghub-zookeeper | zstd-3 | 978507.0 | 30.41229751039083 | 31549180.0 | 419163941.0 |
| loghub-zookeeper | zstd-9 | 693507.0 | 42.91037581451953 | 176392827.0 | 426532828.0 |
| smoke-standard-10000 | duckdb-raw | 798720.0 | 5.806281300080128 | 144323430.0 | 1478434030.0 |
| smoke-standard-10000 | gzip-6 | 459351.0 | 10.095968007036014 | 45329937.0 | 206864528.0 |
| smoke-standard-10000 | jsonl | 4637593.0 | 1.0 | 700.0 | 205081923.0 |
| smoke-standard-10000 | parquet-raw-zstd | 250620.0 | 18.50448088739925 | 12385466.0 | 1542514677.0 |
| smoke-standard-10000 | tracefold-auto-zstd9 | 218127.0 | 21.26097640365475 | 360293062.0 | 810707511.0 |
| smoke-standard-10000 | views-parquet-zstd | 1377243.0 | 3.3673019213021957 | 76464822.0 | 1695591942.0 |
| smoke-standard-10000 | zstd-3 | 456689.0 | 10.154816516272561 | 15881688.0 | 209219318.0 |
| smoke-standard-10000 | zstd-9 | 390177.0 | 11.885869746294631 | 65368974.0 | 205243264.0 |

## Failures

No failures were recorded in this result set.
