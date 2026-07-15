#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CAP_BYTES="${TRACEFOLD_MAX_SOURCE_BYTES:-1073741824}"
COMMIT="$(git rev-parse HEAD 2>/dev/null || printf unknown)"

mkdir -p data/normalized artifacts results/raw-work
TRACEFOLD_GIT_COMMIT="$COMMIT" cargo build --release --locked
target/release/tracefold bench fetch \
  --manifest benches/corpora.toml --max-source-bytes "$CAP_BYTES"
target/release/tracefold normalize \
  --adapter loghub-zookeeper \
  --input data/raw/loghub-zookeeper.log \
  --output data/normalized/zookeeper.jsonl
target/release/tracefold normalize \
  --adapter loghub-bgl \
  --input data/raw/loghub-bgl.log \
  --output data/normalized/bgl.jsonl
target/release/tracefold bench public \
  --output results/raw-work/public.jsonl \
  --max-source-bytes "$CAP_BYTES"

uv sync --project scripts/report --locked
uv run --project scripts/report tracefold-baselines \
  --input data/normalized/zookeeper.jsonl \
  --contract contracts/telemetry-v1.toml \
  --results results/raw-work/public.jsonl \
  --output-dir artifacts/baselines-zookeeper \
  --dataset loghub-zookeeper
uv run --project scripts/report tracefold-baselines \
  --input data/normalized/bgl.jsonl \
  --contract contracts/telemetry-v1.toml \
  --results results/raw-work/public.jsonl \
  --output-dir artifacts/baselines-bgl \
  --dataset loghub-bgl

uv run --project scripts/report tracefold-report \
  --input results/raw-work \
  --output results
pnpm install --frozen-lockfile
pnpm --dir site build
