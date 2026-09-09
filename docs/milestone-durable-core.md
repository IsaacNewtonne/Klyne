# Durable core checkpoint — 2026-09-09

## Inspection

Baseline `861124e` contained 17 tracked files, two Rust crates, two unit tests, a Bash demo, and an architecture proposal. No TODO/FIXME markers or repository AGENTS.md were found. The only initial working-tree difference was the demo script executable bit. Existing code was preserved and extended.

The original environment report described a previous Linux environment. This session runs Windows/MSVC. Rustup installed the pinned 1.98.1 toolchain after filesystem elevation. Registry access needed network elevation. Baseline tests also passed on preinstalled Rust 1.95.0; baseline formatting failed.

## Design decisions

- Synchronous `rusqlite` fits the existing synchronous runtime. An async SQL pool adds complexity without current concurrency demand. Bundled SQLite makes library availability reproducible; Serde provides checkpoint serialization. Cargo.lock records exact dependencies.
- SQLite WAL with `synchronous=FULL`; checkpoint payload and checkpoint event commit together. See [SQLite WAL](https://www.sqlite.org/wal.html) and [rusqlite Connection](https://docs.rs/rusqlite/0.40.2/rusqlite/struct.Connection.html).
- An OS file lock is held for the store lifetime and releases after process kill. It excludes cooperating executors using the same database path, not hostile processes or alternate hard-link aliases.
- Persist pending action before invocation; persist observation/history before another decision. Refuse replay of pending actions on restart. No exactly-once side-effect claim.
- Persist step consumption before model invocation. A crash can consume a step without an action; restart never resets the budget.
- Runtime independently reads and compares requested file contents before accepting model completion. Criteria still use a narrow objective grammar; a general verifier interface remains planned.
- Default shell capability revoked: executable allowlisting alone permitted reads outside the workspace. The shell implementation remains for future supervised execution.

## Evidence

- CLI creation and terminal resume demonstrated; file bytes independently inspected.
- Tests kill a worker after a saved write observation, restart it, verify contents, and assert exactly one write invocation.
- All captured checkpoint boundaries tested: non-pending checkpoints resume; pending checkpoints are refused.
- Tests cover false model completion, traversal, reserved paths, shell denial, persistence failure, budgets, competing executors, unknown schemas and append-only triggers.
- Windows junction and Unix symlink tests are platform-specific; source coverage is not proof of execution on another OS.

Initial debug benchmark `file-core-v1`, Windows/MSVC Rust 1.98.1:

| Metric | Observed |
| --- | ---: |
| Positive tasks independently verified | 10 / 10 |
| Negative traversal cases denied | 10 / 10 |
| Tool calls | 30 |
| Local deterministic model decisions | 40 |
| Runtime, excluding compilation | 0.5175284 seconds |
| Model tokens / model cost | 0 / $0 |
| Escaped file created | false |

Single measured sample, not a throughput guarantee or intelligence comparison. Command: `cargo run -p harness-core --example benchmark`.

## Remaining work

Phase 1 is a tested file-action subset; usable supervised shell execution is partial. Phase 2 is a checkpoint subset, not a complete durable scheduler. Next: typed success criteria independent of the adapter, stable action IDs and reconciliation, bounded process supervisor, goal/task DAG, atomic file writes and filesystem capability handles. Real providers and autonomous coding follow those boundaries.

No model-provider, GUI, browser, host reboot, or general coding benchmark success is claimed.
