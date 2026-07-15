from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("svg")
import matplotlib.pyplot as plt

matplotlib.rcParams["svg.hashsalt"] = "tracefold-v1"


def _finish(fig: plt.Figure, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fig.tight_layout()
    fig.savefig(path, format="svg", metadata={"Date": None})
    plt.close(fig)


def _empty(path: Path, title: str, message: str = "No successful measurements") -> None:
    fig, axis = plt.subplots(figsize=(8, 4.5))
    axis.set_title(title)
    axis.text(0.5, 0.5, message, ha="center", va="center", transform=axis.transAxes)
    axis.set_axis_off()
    _finish(fig, path)


def bars(rows: list[dict[str, Any]], path: Path, title: str, field: str) -> None:
    usable = [row for row in rows if row.get("success") and row.get(field) is not None]
    if not usable:
        return _empty(path, title)
    labels = [f"{row['dataset']}\n{row['baseline']}" for row in usable]
    values = [float(row[field]) for row in usable]
    fig, axis = plt.subplots(figsize=(max(8, len(labels) * 0.72), 5))
    axis.bar(range(len(values)), values, color="#246bce")
    axis.set_title(title)
    axis.set_xticks(range(len(labels)), labels, rotation=45, ha="right", fontsize=8)
    axis.set_ylabel(field.replace("_", " "))
    axis.grid(axis="y", alpha=0.2)
    _finish(fig, path)


def scatter(rows: list[dict[str, Any]], path: Path) -> None:
    usable = [
        row
        for row in rows
        if row.get("success")
        and row.get("archive_bytes") is not None
        and row.get("query_batch_wall_ns") is not None
    ]
    if not usable:
        return _empty(path, "Storage–query latency frontier")
    fig, axis = plt.subplots(figsize=(8, 5))
    for row in usable:
        axis.scatter(row["archive_bytes"], row["query_batch_wall_ns"], s=55)
        axis.annotate(row["baseline"], (row["archive_bytes"], row["query_batch_wall_ns"]), fontsize=8)
    axis.set_title("Storage–query latency frontier")
    axis.set_xlabel("archive bytes")
    axis.set_ylabel("query batch wall time (ns)")
    axis.grid(alpha=0.2)
    _finish(fig, path)


def failure_summary(rows: list[dict[str, Any]], path: Path) -> None:
    failures: dict[str, int] = {}
    for row in rows:
        if not row.get("success"):
            kind = row.get("failure_kind") or "unspecified"
            failures[kind] = failures.get(kind, 0) + 1
    if not failures:
        return _empty(path, "Failure and mismatch summary", "No recorded failures")
    fig, axis = plt.subplots(figsize=(8, 4.5))
    axis.bar(failures.keys(), failures.values(), color="#b54040")
    axis.set_title("Failure and mismatch summary")
    axis.set_ylabel("attempts")
    axis.tick_params(axis="x", rotation=30)
    _finish(fig, path)


def generate_all(rows: list[dict[str, Any]], output: Path) -> list[str]:
    chart_dir = output / "site-data" / "charts"
    specifications: list[tuple[str, Callable[[], None]]] = [
        ("archive-bytes.svg", lambda: bars(rows, chart_dir / "archive-bytes.svg", "Archive bytes by corpus and baseline", "archive_bytes")),
        ("compression-ratio.svg", lambda: bars(rows, chart_dir / "compression-ratio.svg", "Compression ratio", "compression_ratio")),
        ("pareto.svg", lambda: scatter(rows, chart_dir / "pareto.svg")),
        ("encode-throughput.svg", lambda: bars(rows, chart_dir / "encode-throughput.svg", "Encode throughput", "throughput_mib_s")),
        ("peak-rss.svg", lambda: bars(rows, chart_dir / "peak-rss.svg", "Peak RSS", "peak_rss_bytes")),
        ("retention-frontier.svg", lambda: bars(rows, chart_dir / "retention-frontier.svg", "Raw recoverability", "raw_recoverability")),
        ("bucket-frontier.svg", lambda: bars(rows, chart_dir / "bucket-frontier.svg", "Bucket-width frontier", "archive_bytes")),
        ("scale-cardinality.svg", lambda: bars(rows, chart_dir / "scale-cardinality.svg", "Scale and cardinality", "bytes_per_event")),
        ("error-retention.svg", lambda: bars(rows, chart_dir / "error-retention.svg", "Error-retention cost", "archive_bytes")),
        ("format-ablation.svg", lambda: bars(rows, chart_dir / "format-ablation.svg", "Semantic Parquet versus custom format", "archive_bytes")),
        ("failures.svg", lambda: failure_summary(rows, chart_dir / "failures.svg")),
    ]
    for _, generate in specifications:
        generate()
    return [name for name, _ in specifications]
