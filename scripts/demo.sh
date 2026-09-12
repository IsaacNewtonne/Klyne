#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
rm -rf workspace/demo
mkdir -p workspace/demo
cargo run -p harness-cli -- \
  --workspace workspace/demo \
  --objective "create file hello.txt with content hello autonomous harness"
printf '\n--- resulting file ---\n'
cat workspace/demo/hello.txt
printf '\n--- event log ---\n'
cargo run -p harness-cli -- --workspace workspace/demo --events
