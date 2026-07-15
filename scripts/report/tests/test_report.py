from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from tracefold_report.model import load_rows
from tracefold_report.report import build
from tracefold_report.stats import bootstrap_median_ratio, median
from tracefold_report.baselines import _query_is_legal


class ReportTests(unittest.TestCase):
    def test_statistics_are_deterministic(self) -> None:
        self.assertEqual(median([3, 1, 2]), 2.0)
        self.assertEqual(
            bootstrap_median_ratio([2, 2, 2], [1, 1, 1], samples=100),
            (2.0, 2.0),
        )

    def test_report_preserves_failures(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "rows.jsonl"
            base = {
                "schema_version": 1,
                "run_id": "run",
                "timestamp": "2026-01-01T00:00:00Z",
                "dataset": "fixture",
                "baseline": "tracefold",
                "size_limit_bytes": 1024,
                "source_within_limit": True,
                "source_bytes": 10,
                "success": False,
                "failure_kind": "timeout",
                "error": "timed out",
            }
            source.write_text(json.dumps(base) + "\n")
            output = root / "output"
            summary = build(load_rows(source), output)
            self.assertEqual(summary["failed_attempts"], 1)
            self.assertIn("timeout", (output / "summary.md").read_text())
            self.assertTrue((output / "site-data/publication.json").exists())

    def test_python_baseline_enforces_contract_boundaries(self) -> None:
        contract = {
            "time_bucket": "1m",
            "families": [
                {
                    "name": "volume",
                    "dimensions": ["service"],
                    "measures": [{"field": "*", "op": "count"}],
                }
            ],
        }
        legal = {
            "family": "volume",
            "start_ns": 0,
            "end_ns": 60_000_000_000,
            "filters": {},
            "group_by": ["service"],
            "measures": ["count"],
        }
        self.assertTrue(_query_is_legal(legal, contract))
        illegal = dict(legal, group_by=["host"])
        self.assertFalse(_query_is_legal(illegal, contract))


if __name__ == "__main__":
    unittest.main()
