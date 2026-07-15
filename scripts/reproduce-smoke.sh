#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CAP_BYTES="${TRACEFOLD_MAX_SOURCE_BYTES:-1073741824}"
EVENTS="${TRACEFOLD_SMOKE_EVENTS:-10000}"
COMMIT="$(git rev-parse HEAD 2>/dev/null || printf unknown)"

mkdir -p data/generated artifacts/baselines-smoke results/raw-work
TRACEFOLD_GIT_COMMIT="$COMMIT" cargo build --release --locked
target/release/tracefold generate \
  --scenario standard --events "$EVENTS" --seed 7 \
  --output data/generated/smoke.jsonl
target/release/tracefold bench canonical \
  --input data/generated/smoke.jsonl \
  --dataset "smoke-standard-${EVENTS}" \
  --output results/raw-work/smoke.jsonl \
  --max-source-bytes "$CAP_BYTES"

uv sync --project scripts/report --locked
uv run --project scripts/report tracefold-baselines \
  --input data/generated/smoke.jsonl \
  --contract contracts/telemetry-v1.toml \
  --results results/raw-work/smoke.jsonl \
  --output-dir artifacts/baselines-smoke
uv run --project scripts/report tracefold-report \
  --input results/raw-work/smoke.jsonl \
  --output results

pnpm install --frozen-lockfile
pnpm --dir site build
