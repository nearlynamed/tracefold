from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import shutil
import time
import tomllib
from pathlib import Path
from typing import Any

import duckdb


IDENTIFIER = re.compile(r"^[a-z_][a-z0-9_]*$")


def _identifier(value: str) -> str:
    if not IDENTIFIER.fullmatch(value):
        raise ValueError(f"unsafe or unsupported SQL identifier: {value}")
    return f'"{value}"'


def _literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def _measure_name(measure: dict[str, Any]) -> str:
    return "count" if measure["op"] == "count" else f'{measure["field"]}:{measure["op"]}'


def _duration_ns(value: str) -> int:
    if value == "0":
        return 0
    match = re.fullmatch(r"([1-9][0-9]*)(ns|us|ms|s|m|h|d)", value)
    if not match:
        raise ValueError(f"invalid duration: {value}")
    multipliers = {
        "ns": 1,
        "us": 1_000,
        "ms": 1_000_000,
        "s": 1_000_000_000,
        "m": 60_000_000_000,
        "h": 3_600_000_000_000,
        "d": 86_400_000_000_000,
    }
    return int(match.group(1)) * multipliers[match.group(2)]


def _family(contract: dict[str, Any], name: str) -> dict[str, Any]:
    return next(family for family in contract["families"] if family["name"] == name)


def _query_is_legal(query: dict[str, Any], contract: dict[str, Any]) -> bool:
    try:
        family = _family(contract, query["family"])
    except StopIteration:
        return False
    width = _duration_ns(contract["time_bucket"])
    if (
        query["start_ns"] >= query["end_ns"]
        or query["start_ns"] % width
        or query["end_ns"] % width
    ):
        return False
    dimensions = set(family["dimensions"])
    if len(query["group_by"]) != len(set(query["group_by"])):
        return False
    if any(field not in dimensions for field in query["group_by"]):
        return False
    if any(field not in dimensions or not values for field, values in query["filters"].items()):
        return False
    available = {_measure_name(measure) for measure in family["measures"]}
    return len(query["measures"]) == len(set(query["measures"])) and all(
        measure in available for measure in query["measures"]
    )


def _filter_sql(filters: dict[str, list[Any]]) -> list[str]:
    clauses: list[str] = []
    for field, values in filters.items():
        column = _identifier(field)
        strings = [_literal(value) for value in values if value is not None]
        alternatives = []
        if strings:
            alternatives.append(f"{column} IN ({','.join(strings)})")
        if any(value is None for value in values):
            alternatives.append(f"{column} IS NULL")
        clauses.append("(" + " OR ".join(alternatives) + ")")
    return clauses


def _histogram_raw(field: str, bounds: list[int]) -> str:
    column = _identifier(field)
    expressions = []
    for index in range(len(bounds) + 1):
        if index == 0:
            predicate = f"{column} < {bounds[0]}"
        elif index == len(bounds):
            predicate = f"{column} >= {bounds[-1]}"
        else:
            predicate = f"{column} >= {bounds[index - 1]} AND {column} < {bounds[index]}"
        expressions.append(f"count(*) FILTER (WHERE {column} IS NOT NULL AND {predicate})")
    return "[" + ",".join(expressions) + "]"


def _raw_query_sql(
    query: dict[str, Any], contract: dict[str, Any], source: str
) -> tuple[str, list[dict[str, Any]]]:
    family = _family(contract, query["family"])
    width = _duration_ns(contract["time_bucket"])
    bucket = f'(timestamp_ns - (((timestamp_ns % {width}) + {width}) % {width}))'
    group_by = query["group_by"]
    selected = query["measures"] or [_measure_name(measure) for measure in family["measures"]]
    expressions = [f"{bucket} AS bucket_ns"]
    expressions.extend(_identifier(field) for field in group_by)
    descriptors: list[dict[str, Any]] = []
    for index, measure in enumerate(family["measures"]):
        name = _measure_name(measure)
        if name not in selected:
            continue
        op = measure["op"]
        field = measure["field"]
        alias = f"metric_{index}"
        if op == "count":
            expression = "count(*)"
            value_type = "count"
        elif op == "count_present":
            expression = f"count({_identifier(field)})"
            value_type = "count"
        elif op == "sum":
            expression = f"sum({_identifier(field)})"
            value_type = "integer"
        elif op == "min":
            expression = f"min({_identifier(field)})"
            value_type = "integer"
        elif op == "max":
            expression = f"max({_identifier(field)})"
            value_type = "integer"
        elif op == "histogram":
            expression = _histogram_raw(field, measure["bounds"])
            value_type = "histogram"
        else:
            raise ValueError(f"unsupported operation: {op}")
        expressions.append(f"{expression} AS {alias}")
        descriptors.append({"name": name, "type": value_type, "alias": alias})
    where = [
        f'timestamp_ns >= {int(query["start_ns"])}',
        f'timestamp_ns < {int(query["end_ns"])}',
        *_filter_sql(query["filters"]),
    ]
    groups = ["bucket_ns", *(_identifier(field) for field in group_by)]
    sql = (
        f"SELECT {','.join(expressions)} FROM {source} "
        f"WHERE {' AND '.join(where)} GROUP BY {','.join(groups)}"
    )
    return sql, descriptors


def _semantic_query_sql(
    query: dict[str, Any], contract: dict[str, Any], path: Path
) -> tuple[str, list[dict[str, Any]]]:
    family = _family(contract, query["family"])
    selected = query["measures"] or [_measure_name(measure) for measure in family["measures"]]
    expressions = ["bucket_ns"]
    expressions.extend(_identifier(field) for field in query["group_by"])
    descriptors: list[dict[str, Any]] = []
    for index, measure in enumerate(family["measures"]):
        name = _measure_name(measure)
        if name not in selected:
            continue
        op = measure["op"]
        alias = f"metric_{index}"
        if op == "count":
            expression = f"sum(m_{index}_count)"
            value_type = "count"
        elif op == "count_present":
            expression = f"sum(m_{index}_present)"
            value_type = "count"
        elif op == "sum":
            expression = f"CASE WHEN sum(m_{index}_present) > 0 THEN sum(m_{index}_sum) END"
            value_type = "integer"
        elif op == "min":
            expression = f"min(m_{index}_min)"
            value_type = "integer"
        elif op == "max":
            expression = f"max(m_{index}_max)"
            value_type = "integer"
        elif op == "histogram":
            expression = "[" + ",".join(
                f"sum(m_{index}_hist_{bin_index})"
                for bin_index in range(len(measure["bounds"]) + 1)
            ) + "]"
            value_type = "histogram"
        else:
            raise ValueError(f"unsupported operation: {op}")
        expressions.append(f"{expression} AS {alias}")
        descriptors.append({"name": name, "type": value_type, "alias": alias})
    where = [
        f'bucket_ns >= {int(query["start_ns"])}',
        f'bucket_ns < {int(query["end_ns"])}',
        *_filter_sql(query["filters"]),
    ]
    groups = ["bucket_ns", *(_identifier(field) for field in query["group_by"])]
    source = f"read_parquet({_literal(str(path))})"
    sql = (
        f"SELECT {','.join(expressions)} FROM {source} "
        f"WHERE {' AND '.join(where)} GROUP BY {','.join(groups)}"
    )
    return sql, descriptors


def _rows(
    cursor: duckdb.DuckDBPyConnection,
    group_by: list[str],
    descriptors: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    names = [column[0] for column in cursor.description]
    output = []
    for values in cursor.fetchall():
        row = dict(zip(names, values, strict=True))
        metrics = {}
        for descriptor in descriptors:
            value = row[descriptor["alias"]]
            if descriptor["type"] == "histogram":
                value = [int(item) for item in value]
            elif value is not None:
                value = int(value)
            metrics[descriptor["name"]] = {
                "type": descriptor["type"],
                "value": value,
            }
        output.append(
            {
                "bucket_ns": int(row["bucket_ns"]),
                "dimensions": {field: row[field] for field in group_by},
                "values": metrics,
            }
        )
    output.sort(
        key=lambda row: (
            row["bucket_ns"],
            *(
                (0, "") if row["dimensions"][field] is None else (1, row["dimensions"][field])
                for field in group_by
            ),
        )
    )
    return output


def _hash_rows(rows: list[dict[str, Any]]) -> str:
    payload = json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def _execute_workload(
    connection: duckdb.DuckDBPyConnection,
    workload: list[dict[str, Any]],
    expected: list[str],
    contract: dict[str, Any],
    source: str | Path,
    *,
    semantic: bool,
) -> tuple[int, int]:
    started = time.perf_counter_ns()
    mismatches = 0
    for query, expected_hash in zip(workload, expected, strict=True):
        if semantic:
            path = Path(source) / f'{query["family"]}.parquet'
            sql, descriptors = _semantic_query_sql(query, contract, path)
        else:
            sql, descriptors = _raw_query_sql(query, contract, str(source))
        rows = _rows(connection.execute(sql), query["group_by"], descriptors)
        mismatches += int(_hash_rows(rows) != expected_hash)
    return time.perf_counter_ns() - started, mismatches


def _create_semantic(
    connection: duckdb.DuckDBPyConnection,
    contract: dict[str, Any],
    output: Path,
) -> tuple[int, int]:
    started = time.perf_counter_ns()
    output.mkdir(parents=True, exist_ok=True)
    views = output / "views"
    dictionaries = output / "dictionaries"
    raw = output / "raw"
    views.mkdir()
    dictionaries.mkdir()
    raw.mkdir()
    width = _duration_ns(contract["time_bucket"])
    bucket = f'(timestamp_ns - (((timestamp_ns % {width}) + {width}) % {width}))'
    dimensions = sorted(
        {dimension for family in contract["families"] for dimension in family["dimensions"]}
    )
    for dimension in dimensions:
        target = dictionaries / f"{dimension}.parquet"
        connection.execute(
            f"COPY (SELECT DISTINCT {_identifier(dimension)} AS value FROM events ORDER BY value NULLS FIRST) TO ? (FORMAT PARQUET, COMPRESSION ZSTD)",
            [str(target)],
        )
    for family in contract["families"]:
        expressions = [f"{bucket} AS bucket_ns"]
        expressions.extend(_identifier(field) for field in family["dimensions"])
        for index, measure in enumerate(family["measures"]):
            expressions.append(f"count(*) AS m_{index}_count")
            if measure["op"] == "count":
                continue
            field = _identifier(measure["field"])
            expressions.extend(
                [
                    f"count({field}) AS m_{index}_present",
                    f"coalesce(sum({field}), 0) AS m_{index}_sum",
                    f"min({field}) AS m_{index}_min",
                    f"max({field}) AS m_{index}_max",
                ]
            )
            if measure["op"] == "histogram":
                bounds = measure["bounds"]
                for bin_index in range(len(bounds) + 1):
                    if bin_index == 0:
                        predicate = f"{field} < {bounds[0]}"
                    elif bin_index == len(bounds):
                        predicate = f"{field} >= {bounds[-1]}"
                    else:
                        predicate = f"{field} >= {bounds[bin_index - 1]} AND {field} < {bounds[bin_index]}"
                    expressions.append(
                        f"count(*) FILTER (WHERE {field} IS NOT NULL AND {predicate}) AS m_{index}_hist_{bin_index}"
                    )
        groups = ["bucket_ns", *(_identifier(field) for field in family["dimensions"])]
        target = views / f'{family["name"]}.parquet'
        connection.execute(
            f"COPY (SELECT {','.join(expressions)} FROM events GROUP BY {','.join(groups)}) TO ? (FORMAT PARQUET, COMPRESSION ZSTD)",
            [str(target)],
        )
    max_timestamp = int(connection.execute("SELECT max(timestamp_ns) FROM events").fetchone()[0])
    recent = contract["retention"]["recent"]
    cutoff = -9_223_372_036_854_775_808 if recent == "all" else max_timestamp - _duration_ns(recent)
    error_severities = ",".join(_literal(value) for value in contract["retention"]["error_severities"])
    error_statuses = ",".join(_literal(value) for value in contract["retention"]["error_statuses"])
    is_error = f"(severity IN ({error_severities}) OR status IN ({error_statuses}))"
    connection.execute(
        f"COPY (SELECT * FROM events WHERE timestamp_ns >= {cutoff} AND NOT {is_error}) TO ? (FORMAT PARQUET, COMPRESSION ZSTD)",
        [str(raw / "recent.parquet")],
    )
    connection.execute(
        f"COPY (SELECT * FROM events WHERE {is_error}) TO ? (FORMAT PARQUET, COMPRESSION ZSTD)",
        [str(raw / "errors.parquet")],
    )
    retained = int(
        connection.execute(
            f"SELECT count(*) FROM events WHERE {is_error} OR timestamp_ns >= {cutoff}"
        ).fetchone()[0]
    )
    (output / "contract.json").write_text(json.dumps(contract, sort_keys=True, separators=(",", ":")) + "\n")
    return time.perf_counter_ns() - started, retained


def _directory_size(path: Path) -> int:
    return sum(item.stat().st_size for item in path.rglob("*") if item.is_file())


def _directory_hash(path: Path) -> str:
    digest = hashlib.sha256()
    for item in sorted(item for item in path.rglob("*") if item.is_file()):
        digest.update(item.relative_to(path).as_posix().encode())
        digest.update(b"\0")
        digest.update(item.read_bytes())
    return digest.hexdigest()


def _ensure_contract_columns(
    connection: duckdb.DuckDBPyConnection, contract: dict[str, Any]
) -> None:
    existing = {
        row[1] for row in connection.execute("PRAGMA table_info('events')").fetchall()
    }
    dimensions = {
        dimension
        for family in contract["families"]
        for dimension in family["dimensions"]
    }
    measures = {
        measure["field"]
        for family in contract["families"]
        for measure in family["measures"]
        if measure["field"] != "*"
    }
    for column in sorted(dimensions - existing):
        connection.execute(f"ALTER TABLE events ADD COLUMN {_identifier(column)} VARCHAR")
    for column in sorted(measures - existing):
        connection.execute(f"ALTER TABLE events ADD COLUMN {_identifier(column)} BIGINT")


def _row(
    template: dict[str, Any],
    baseline: str,
    archive_bytes: int,
    encode_ns: int,
    query_ns: int,
    mismatches: int,
    order: int,
    *,
    recoverability: float,
    archive_hash: str | None = None,
) -> dict[str, Any]:
    row = copy.deepcopy(template)
    normalized = int(row["normalized_bytes"])
    records = int(row["record_count"])
    row.update(
        {
            "baseline": baseline,
            "archive_bytes": archive_bytes,
            "compression_ratio": normalized / max(archive_bytes, 1),
            "bytes_per_event": archive_bytes / max(records, 1),
            "encode_wall_ns": encode_ns,
            "throughput_mib_s": normalized / 1_048_576 / max(encode_ns / 1_000_000_000, 1e-9),
            "query_batch_wall_ns": query_ns,
            "timing_mode": "single-process-warm",
            "query_count": len(row["query_workload"]),
            "semantic_mismatch_count": mismatches,
            "raw_recoverability": recoverability,
            "randomized_order": order,
            "archive_hash": archive_hash,
            "layout": "semantic" if baseline == "views-parquet-zstd" else "raw",
            "codec": "zstd",
            "success": mismatches == 0,
            "failure_kind": None if mismatches == 0 else "semantic_mismatch",
            "error": None if mismatches == 0 else f"{mismatches} query results differed from the Rust oracle",
        }
    )
    row["query_workload"] = None
    row["illegal_query_workload"] = None
    row["oracle_result_sha256"] = None
    return row


def build_baselines(
    input_path: Path,
    contract_path: Path,
    results_path: Path,
    output_dir: Path,
    dataset: str | None = None,
) -> list[dict[str, Any]]:
    templates = [json.loads(line) for line in results_path.read_text().splitlines() if line.strip()]
    template = next(
        row
        for row in templates
        if row["baseline"] == "tracefold-auto-zstd9"
        and row.get("query_workload")
        and (dataset is None or row["dataset"] == dataset)
    )
    contract = tomllib.loads(contract_path.read_text())
    workload = template["query_workload"]
    expected = template["oracle_result_sha256"]
    rejection_count = sum(
        not _query_is_legal(query, contract)
        for query in template.get("illegal_query_workload") or []
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    database = output_dir / "raw.duckdb"
    parquet = output_dir / "raw.parquet"
    semantic = output_dir / "views-parquet-zstd"
    for path in (database, parquet):
        path.unlink(missing_ok=True)
    shutil.rmtree(semantic, ignore_errors=True)

    duck_started = time.perf_counter_ns()
    connection = duckdb.connect(str(database))
    connection.execute(
        "CREATE TABLE events AS SELECT * FROM read_json_auto(?, format='newline_delimited', maximum_object_size=16777216)",
        [str(input_path)],
    )
    _ensure_contract_columns(connection, contract)
    connection.execute("CHECKPOINT")
    duck_encode_ns = time.perf_counter_ns() - duck_started

    parquet_started = time.perf_counter_ns()
    connection.execute("COPY events TO ? (FORMAT PARQUET, COMPRESSION ZSTD)", [str(parquet)])
    parquet_encode_ns = time.perf_counter_ns() - parquet_started

    semantic_encode_ns, retained = _create_semantic(connection, contract, semantic)
    duck_query_ns, duck_mismatches = _execute_workload(
        connection, workload, expected, contract, "events", semantic=False
    )
    parquet_query_ns, parquet_mismatches = _execute_workload(
        connection,
        workload,
        expected,
        contract,
        f"read_parquet({_literal(str(parquet))})",
        semantic=False,
    )
    semantic_query_ns, semantic_mismatches = _execute_workload(
        connection, workload, expected, contract, semantic / "views", semantic=True
    )
    connection.close()
    rows = [
        _row(
            template,
            "duckdb-raw",
            database.stat().st_size,
            duck_encode_ns,
            duck_query_ns,
            duck_mismatches,
            5,
            recoverability=1.0,
            archive_hash=hashlib.sha256(database.read_bytes()).hexdigest(),
        ),
        _row(
            template,
            "parquet-raw-zstd",
            parquet.stat().st_size,
            parquet_encode_ns,
            parquet_query_ns,
            parquet_mismatches,
            6,
            recoverability=1.0,
            archive_hash=hashlib.sha256(parquet.read_bytes()).hexdigest(),
        ),
        _row(
            template,
            "views-parquet-zstd",
            _directory_size(semantic),
            semantic_encode_ns,
            semantic_query_ns,
            semantic_mismatches,
            7,
            recoverability=retained / max(int(template["record_count"]), 1),
            archive_hash=_directory_hash(semantic),
        ),
    ]
    for row in rows:
        row["explicit_rejection_count"] = rejection_count
    with results_path.open("a") as stream:
        for row in rows:
            stream.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")
    return rows


def main() -> None:
    parser = argparse.ArgumentParser(description="Build and verify DuckDB/Parquet TraceFold baselines")
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--contract", required=True, type=Path)
    parser.add_argument("--results", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--dataset")
    args = parser.parse_args()
    rows = build_baselines(
        args.input,
        args.contract,
        args.results,
        args.output_dir,
        args.dataset,
    )
    print(
        json.dumps(
            {
                "schema_version": 1,
                "baselines": len(rows),
                "successful": sum(row["success"] for row in rows),
                "mismatches": sum(row["semantic_mismatch_count"] for row in rows),
            },
            sort_keys=True,
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    main()
