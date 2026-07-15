from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REQUIRED = {
    "schema_version",
    "run_id",
    "timestamp",
    "dataset",
    "baseline",
    "success",
    "size_limit_bytes",
    "source_within_limit",
}


@dataclass(frozen=True)
class LoadedRows:
    rows: list[dict[str, Any]]
    files: list[Path]


def load_rows(source: Path) -> LoadedRows:
    files = [source] if source.is_file() else sorted(source.rglob("*.jsonl"))
    rows: list[dict[str, Any]] = []
    for path in files:
        for line_number, line in enumerate(path.read_text().splitlines(), 1):
            if not line.strip():
                continue
            row = json.loads(line)
            missing = REQUIRED - row.keys()
            if missing:
                raise ValueError(f"{path}:{line_number}: missing {sorted(missing)}")
            if row["schema_version"] != 1:
                raise ValueError(
                    f"{path}:{line_number}: unsupported schema {row['schema_version']}"
                )
            if row["source_bytes"] is not None and row["source_bytes"] > row["size_limit_bytes"]:
                if row["success"]:
                    raise ValueError(f"{path}:{line_number}: oversized successful row")
            rows.append(row)
    if not rows:
        raise ValueError(f"no benchmark rows found in {source}")
    return LoadedRows(rows, files)

