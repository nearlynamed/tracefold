from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
from collections import defaultdict
from pathlib import Path
from typing import Any

from .charts import generate_all
from .model import LoadedRows
from .stats import median, percentile


def _stable_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")


def _git_commit() -> str:
    try:
        return subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def build(loaded: LoadedRows, output: Path) -> dict[str, Any]:
    output.mkdir(parents=True, exist_ok=True)
    rows = loaded.rows
    successful = [row for row in rows if row["success"]]
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in successful:
        grouped[(row["dataset"], row["baseline"])].append(row)
    table = []
    for (dataset, baseline), values in sorted(grouped.items()):
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
        "attempts": len(rows),
        "successful_attempts": len(successful),
        "failed_attempts": len(failures),
        "datasets": sorted({row["dataset"] for row in rows}),
        "baselines": sorted({row["baseline"] for row in rows}),
        "max_source_bytes": max(row["size_limit_bytes"] for row in rows),
        "table": table,
        "failures": failures,
        "charts": charts,
    }
    site_data = output / "site-data"
    _stable_json(site_data / "summary.json", summary)
    _stable_json(site_data / "tables" / "primary.json", table)
    methodology = {
        "schema_version": 1,
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
        "publication_commit": _git_commit(),
        "raw_results": raw_manifest,
        "evidence": evidence,
    }
    _stable_json(site_data / "publication.json", publication)
    _write_markdown(output, summary, publication)
    return summary


def _write_markdown(output: Path, summary: dict[str, Any], publication: dict[str, Any]) -> None:
    lines = [
        "# TraceFold benchmark summary",
        "",
        f"**Status:** {publication['status']}  ",
        f"**By:** {publication['byline']}  ",
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

    paper = f"""# TraceFold: Query-Preserving Compression for Tiered Telemetry Archives

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

All source artifacts are capped at {summary['max_source_bytes']} bytes under the declared downloaded/generated-source basis. Synthetic data is seeded; public data comes from Loghub. Benchmark queries are created after archive finalization.

## Results

The current publication contains {summary['attempts']} attempts: {summary['successful_attempts']} successful and {summary['failed_attempts']} failed. See `summary.md` and the generated figures for the complete per-corpus results.

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
"""
    (output / "paper.md").write_text(paper)

