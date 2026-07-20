from __future__ import annotations

import hashlib
import json
import shutil
from collections import defaultdict
from pathlib import Path
from typing import Any

from .charts import generate_all
from .model import LoadedRows
from .paper import render_paper
from .stats import median, percentile


PRIMARY_BASELINES = {
    "jsonl",
    "gzip-6",
    "zstd-3",
    "zstd-9",
    "duckdb-raw",
    "parquet-raw-zstd",
    "views-parquet-zstd",
    "tracefold-auto-zstd9",
}


def _stable_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")


def build(loaded: LoadedRows, output: Path) -> dict[str, Any]:
    output.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(output / "site-data", ignore_errors=True)
    (output / "paper.md").unlink(missing_ok=True)
    (output / "summary.md").unlink(missing_ok=True)
    rows = loaded.rows
    commits = {row.get("git_commit") for row in rows if row.get("git_commit")}
    if len(commits) > 1:
        raise ValueError(f"benchmark rows contain multiple implementation commits: {sorted(commits)}")
    source_manifest = [
        {
            "name": path.name,
            "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        }
        for path in sorted(loaded.files)
    ]
    snapshot_id = hashlib.sha256(
        json.dumps(source_manifest, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    successful = [row for row in rows if row["success"]]
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in successful:
        grouped[(row["dataset"], row["baseline"])].append(row)
    table = []
    for (dataset, baseline), values in sorted(grouped.items()):
        if baseline not in PRIMARY_BASELINES:
            continue
        table.append(
            {
                "dataset": dataset,
                "baseline": baseline,
                "attempts": len(values),
                "archive_bytes_median": median([row["archive_bytes"] for row in values if row["archive_bytes"] is not None]),
                "compression_ratio_median": median([row["compression_ratio"] for row in values if row["compression_ratio"] is not None]),
                "encode_wall_ns_median": median([row["encode_wall_ns"] for row in values if row["encode_wall_ns"] is not None]),
                "query_wall_ns_median": median([row["query_batch_wall_ns"] for row in values if row["query_batch_wall_ns"] is not None]),
                "encode_wall_ns_p95": percentile([row["encode_wall_ns"] for row in values if row["encode_wall_ns"] is not None], 95),
            }
        )
    charts = generate_all(rows, output)
    failures = [
        {
            "dataset": row["dataset"],
            "baseline": row["baseline"],
            "kind": row.get("failure_kind"),
            "error": row.get("error"),
        }
        for row in rows
        if not row["success"]
    ]
    summary = {
        "schema_version": 1,
        "snapshot_id": snapshot_id,
        "attempts": len(rows),
        "successful_attempts": len(successful),
        "failed_attempts": len(failures),
        "datasets": sorted({row["dataset"] for row in rows}),
        "baselines": sorted({row["baseline"] for row in rows}),
        "max_source_bytes": max(row["size_limit_bytes"] for row in rows),
        "largest_observed_source_bytes": max(
            (row.get("source_bytes") or 0) for row in rows
        ),
        "largest_normalized_bytes": max(
            (row.get("normalized_bytes") or 0) for row in rows
        ),
        "semantic_mismatch_count": sum(
            row.get("semantic_mismatch_count", 0) for row in rows
        ),
        "legal_queries_per_archive": max(
            (row.get("query_count", 0) for row in rows), default=0
        ),
        "illegal_queries_per_archive": max(
            (row.get("explicit_rejection_count", 0) for row in rows), default=0
        ),
        "table": table,
        "failures": failures,
        "charts": charts,
    }
    site_data = output / "site-data"
    _stable_json(site_data / "summary.json", summary)
    _stable_json(site_data / "tables" / "primary.json", table)
    methodology = {
        "schema_version": 1,
        "snapshot_id": snapshot_id,
        "semantic_contract": "Exact declared aggregate query families; recent and error records retained exactly.",
        "cap_basis": "Downloaded public source bytes or generated canonical JSONL bytes.",
        "cap_bytes": summary["max_source_bytes"],
        "anti_cheating": "Query workloads are generated after archive finalization.",
        "timing": "Medians are primary; process-cold does not claim disk-cold cache state.",
    }
    _stable_json(site_data / "methodology.json", methodology)
    raw_dir = site_data / "raw"
    raw_dir.mkdir(parents=True, exist_ok=True)
    raw_manifest = []
    for path in loaded.files:
        target = raw_dir / path.name
        shutil.copyfile(path, target)
        raw_manifest.append(
            {
                "path": f"raw/{target.name}",
                "bytes": target.stat().st_size,
                "sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
            }
        )
    evidence = [
        {
            "id": f"table-primary-{index}",
            "dataset": row["dataset"],
            "baseline": row["baseline"],
        }
        for index, row in enumerate(table)
    ]
    publication = {
        "schema_version": 1,
        "title": "TraceFold: Query-Preserving Compression for Tiered Telemetry Archives",
        "byline": "nearlynamed",
        "status": "Technical report — not peer reviewed",
        "benchmark_commit": next((row.get("git_commit") for row in rows if row.get("git_commit")), "unknown"),
        "snapshot_id": snapshot_id,
        "raw_results": raw_manifest,
        "evidence": evidence,
    }
    _stable_json(site_data / "publication.json", publication)
    _write_markdown(output, summary, publication, rows)
    return summary


def _write_markdown(
    output: Path,
    summary: dict[str, Any],
    publication: dict[str, Any],
    rows: list[dict[str, Any]],
) -> None:
    lines = [
        "# TraceFold benchmark summary",
        "",
        f"**Status:** {publication['status']}",
        f"**By:** {publication['byline']}",
        f"**Snapshot:** `{summary['snapshot_id']}`",
        f"**Attempts:** {summary['attempts']} ({summary['successful_attempts']} successful, {summary['failed_attempts']} failed)",
        "",
        "| Dataset | Baseline | Archive bytes (median) | Compression ratio | Encode time (ns) | Query batch (ns) |",
        "| --- | --- | ---: | ---: | ---: | ---: |",
    ]
    for row in summary["table"]:
        lines.append(
            f"| {row['dataset']} | {row['baseline']} | {row['archive_bytes_median']} | {row['compression_ratio_median']} | {row['encode_wall_ns_median']} | {row['query_wall_ns_median']} |"
        )
    lines.extend(["", "## Failures", ""])
    if summary["failures"]:
        lines.extend(
            f"- `{row['dataset']}` / `{row['baseline']}`: {row['kind']} — {row['error']}"
            for row in summary["failures"]
        )
    else:
        lines.append("No failures were recorded in this result set.")
    (output / "summary.md").write_text("\n".join(lines) + "\n")

    (output / "paper.md").write_text(render_paper(summary, publication, rows))
