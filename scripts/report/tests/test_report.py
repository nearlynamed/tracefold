from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from tracefold_report.model import load_rows
from tracefold_report.report import build
from tracefold_report.stats import bootstrap_median_ratio, median
from tracefold_report.baselines import _query_is_legal
from tracefold_report.charts import bars


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
            self.assertEqual(len(summary["snapshot_id"]), 64)
            self.assertIn("timeout", (output / "summary.md").read_text())
            self.assertTrue((output / "site-data/publication.json").exists())
            publication = json.loads((output / "site-data/publication.json").read_text())
            methodology = json.loads((output / "site-data/methodology.json").read_text())
            self.assertEqual(publication["snapshot_id"], summary["snapshot_id"])
            self.assertEqual(methodology["snapshot_id"], summary["snapshot_id"])
            self.assertNotIn("publication_commit", publication)
            paper = (output / "paper.md").read_text()
            self.assertGreater(len(paper.split()), 2500)
            self.assertIn("## 2. Semantic contract", paper)
            self.assertIn("## 8. Threats to validity", paper)
            self.assertIn("## 11. Reproducibility and provenance", paper)

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

    def test_charts_mark_tracefold_distinctly(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            chart = Path(directory) / "chart.svg"
            bars(
                [
                    {
                        "success": True,
                        "dataset": "fixture",
                        "baseline": "tracefold-auto-zstd9",
                        "archive_bytes": 10,
                    },
                    {
                        "success": True,
                        "dataset": "fixture",
                        "baseline": "parquet-raw-zstd",
                        "archive_bytes": 20,
                    },
                ],
                chart,
                "Fixture",
                "archive_bytes",
            )
            svg = chart.read_text()
            self.assertIn("TraceFold", svg)
            self.assertNotIn("ours", svg.lower())
            self.assertIn("#d64b2a", svg)

    def test_report_rejects_mixed_implementation_commits(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "rows.jsonl"
            rows = [
                {
                    "schema_version": 1,
                    "run_id": f"run-{commit}",
                    "timestamp": "2026-01-01T00:00:00Z",
                    "dataset": "fixture",
                    "baseline": "jsonl",
                    "size_limit_bytes": 1024,
                    "source_within_limit": True,
                    "source_bytes": 10,
                    "success": True,
                    "git_commit": commit,
                }
                for commit in ("a", "b")
            ]
            source.write_text("".join(json.dumps(row) + "\n" for row in rows))
            with self.assertRaisesRegex(ValueError, "multiple implementation commits"):
                build(load_rows(source), root / "output")


if __name__ == "__main__":
    unittest.main()
