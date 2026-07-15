from __future__ import annotations

import argparse
import json
from pathlib import Path

from .model import load_rows
from .report import build


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate TraceFold publication artifacts")
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    summary = build(load_rows(args.input), args.output)
    print(json.dumps(summary, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()

