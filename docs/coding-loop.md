# Deterministic inspect → patch → verify loop

Status: **IMPLEMENTED** for a deterministic scripted agent over one workspace file.
General autonomous coding remains **PLANNED**.

## What changed

- New `SearchFile { path, needle, max_matches }` action in the `workspace_fs` tool:
  bounded substring search (needle 1..1024 bytes, matches 1..50) streamed over files
  up to 64 MiB with 64 KiB chunks and needle overlap, so matches spanning chunk
  boundaries are found. Each match reports byte offset, 1-based line number, and a
  bounded (160-byte) excerpt hint. Excess matches set `truncated` instead of growing
  the observation. Like all file actions it carries permission checks, persisted
  tool-call reservations, and audit records, and appears in `--tools` discovery.
- New `SuccessCriterion` variants with runtime-owned verification:
  `FileDigest { path, sha256 }` (verified by a fresh `HashFile` observation) and
  `FileRange { path, offset, expected }` (verified by a fresh `ReadFileRange`
  observation of exactly `expected.len()` bytes). The verifier binds evidence to
  the exact expected action and a successful observation, and parses the typed
  JSON fields (`sha256`, `text`) rather than trusting summaries.
- New `AgentRuntime::run_with_criterion()` starts a run with an explicit
  machine-checked claim instead of deriving one from objective text. The claim
  persists in the checkpoint and survives resume; budgets, permissions, and
  independent verification are otherwise identical. The CLI grammar is unchanged:
  it still only understands file-creation objectives.
- Reconciliation extended to interrupted read-only actions (`ReadFileRange`,
  `HashFile`, `SearchFile`): they recover with a fresh observation under a new
  tool-call reservation, keeping the stable run-scoped action ID. Interrupted
  `PatchFile` (like shell) stays refused: before/after/conflicting states cannot
  be distinguished from outside, so no reconciler is claimed.

## Verified

New `crates/harness-core/tests/coding_loop.rs` (6 tests on Windows/MSVC):

- A deterministic repair agent fixes `return 41;` → `return 42;` in a controlled
  repository through the runtime: search → range read → hash → digest-guarded
  patch → `Complete`, with independent `FileRange` verification. Exactly 5 tool
  calls (4 agent + 1 runtime-owned) are accounted; `VerificationPassed` and
  `GoalCompleted` are recorded; reopening the terminal run repeats zero events.
- Stale digests and wrong expected spans are rejected with the file untouched;
  a successful patch is applied once and its retry is rejected as stale.
- Search returns exact offsets, line numbers, and excerpts; truncation is
  reported at the match cap; empty/oversized needles, out-of-range match counts,
  traversal, reserved, and missing paths all fail.
- Interrupted range/hash/search checkpoints reconcile with the same action ID
  across records and complete without a duplicate `ToolCalled` entry.
- An interrupted patch refuses reconciliation, leaves bytes identical, and
  preserves pending state; a symlink/junction permission change denies search
  reconciliation and preserves pending state (platform-gated tests).

Full suite after this change: 54 passing (47 prior + 6 loop + 1 verifier unit).
`cargo fmt --check`, `cargo check --locked`, `cargo test --locked`, and
`cargo clippy -- -D warnings` pass. `file-core-v1` (10/10, 10/10 denied) and
`large-files-v1` (8 MiB, independently verified) benchmarks are unchanged.

## Limits (honest)

- The repair agent is a hardcoded script, not intelligence: fixed step order,
  single bug shape, no planning or failure-driven replanning.
- Patch reconciliation is intentionally unimplemented; crashes around patch
  publication still require operator inspection (temp-file cleanup after a kill
  is also unresolved, as documented for large files).
- No compile/test oracle, no goal DAG, no memory: multi-step objectives with
  dependencies are still planned.
