# Memory and context selection

Status: **IMPLEMENTED** as SQLite-backed project, episodic, procedural, and
failure memory with explained lexical retrieval, retention, and
consolidation. Vector embeddings and graph projections remain **PLANNED**;
ranking is transparent keyword scoring, not similarity search.

## What changed

- New `harness-memory` crate. Records carry kind, content, confidence
  (0–1), provenance, millisecond timestamps, salience (0–1), a reinforcement
  counter, and a tombstone pointer. The store is a separate SQLite file with
  WAL + `synchronous=FULL`, like the run store.
- Retrieval is scored and explained: term overlap, confidence, 30-day linear
  recency (floor 0.25), log reinforcement, and an optional provenance boost.
  Every hit returns per-dimension reasons; superseded rows never retrieve
  but stay auditable as tombstones.
- `supersede(old, new)` retires outdated lessons explicitly; unknown,
  already-superseded, and self-supersede attempts are rejected.
- `consolidate()` merges exact-duplicate contents per kind into the earliest
  row (max confidence, summed reinforcement) and tombstones the rest.
- `forget` / `forget_as_of` delete rows older than a cutoff with salience
  below a threshold, plus old tombstones; high-salience rows survive any
  age. The `_as_of` variant takes an explicit clock so retention is testable.
- Benchmark integration: `record_repair` stores an episodic outcome plus a
  procedural `{shape, bug, fix}` JSON mapping, and only for verified
  reports. `recall_fix` parses candidates strictly, matches on shape and bug
  value (never on score alone), enforces a confidence floor, and returns the
  retrieval reasons. The remembered fix becomes the agent's first attempt;
  tests and independent verification still decide, and the normal
  failure-driven loop takes over when memory is wrong.

## Verified

- `harness-memory/tests/memory.rs` (5): relevant procedure outranks
  distractors with reasons attached; superseded rows never retrieve;
  consolidation merges with max confidence and summed reinforcement;
  retention forgets old trivia but keeps salient rows; kind filters and
  limits behave.
- `harness-benchmark/tests/recall.rs` (3), real cargo runs:
  - Primed repeat uses fewer tool calls than the same-task baseline, never
    attempts the wrong fix or the (higher-confidence!) cross-shape
    distractor, and stays oracle-verified.
  - A stale confident lesson fails safe: the remembered fix is tried,
    rejected by test evidence, and the repair loop still completes; the
    lesson is then superseded and stops retrieving.
  - Unverified reports are refused by `record_repair`; unknown shapes and
    over-threshold confidence floors recall nothing.
- Full suite: 88 passing (80 prior + 8 memory). `cargo fmt --check`,
  `cargo check --locked`, `cargo test --locked`,
  `cargo clippy -- -D warnings` pass.

## Limits (honest)

- Lexical scoring only: no embeddings, no vector index, no graph expansion.
  Paraphrased lessons that share no tokens will not match.
- Fix mappings are exact shape+bug JSON; near-miss generalization (same
  shape, different value) is not attempted — the agent falls back to its
  baseline loop instead.
- No promotion rule yet (repeated episodes do not auto-promote to
  procedural); no cross-workspace sharing; memory budget is unbounded
  except through explicit `forget`/`consolidate` calls.
- Retrieval latency over large stores is unmeasured; the index covers kind
  only, so broad queries scan.
