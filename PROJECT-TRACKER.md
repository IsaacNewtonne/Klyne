# Project Tracker — Local-first Autonomous AI Agent Harness

> Living file. Update it on every checkpoint: move rows, refresh counts,
> record the new hash. Status legend: **IMPLEMENTED**, **PARTIAL**,
> **EXPERIMENTAL**, **PLANNED**, **BLOCKED**.
> Living file: update it with every feature checkpoint below. Suite **130 passing** as of 2026-09-09.

## Checkpoint log

| Hash | What landed |
| --- | --- |
| `861124e` | Bootstrap: workspace, runtime loop, model trait, file/shell tools, permission gate, events, read-back verification |
| `308e60e` | Verified SQLite checkpoints and conservative crash recovery |
| `60edbb1` | Typed success criteria and evidence verification separated from models |
| `2c260fd` | Interrupted file-action reconciliation with stable run-scoped IDs |
| `01dcca1` | Tool-call reservations and bound filesystem operations (1 MiB) |
| `4078df8` | Bounded range reads, streaming SHA-256, digest-guarded patches |
| `6b3a9cf` | Tool descriptors/discovery, supervised argv process tool (timeout, output caps, Windows tree kill, explicit grants) |
| `5dfdbfc` | Bounded file search, hash/range criteria, `run_with_criterion`, read-only reconciliation, deterministic repair loop |
| `7c122fb` | `harness-provider`: OpenAI-compatible adapter, strict decisions, secret hygiene, retries, usage accounting, CLI `--openai-compat` |
| `02be314` | `harness-benchmark`: scripted cargo repair tasks, compiler/test oracle, failure-driven repair, restart |
| `280e68e` | Durable goal/task plans, lifecycle, budget amendments, hierarchical scheduling without duplicated effects |
| `6f0782e` | Living project tracker |
| `8d35740` | `harness-memory`: SQLite project/episodic/procedural/failure memory, explained retrieval, retention, consolidation, primed benchmark recall |
| `3819cfe` | Resource budgets (wall/token/cost), repeated-error trip, shutdown, heartbeats, `inspect` module + CLI `--inspect`, provider pricing |

## Roadmap milestones

| # | Milestone | Status | Evidence |
| --- | --- | --- | --- |
| 1 | Single-agent execution boundary (descriptors, supervised process) | **IMPLEMENTED** | `crates/harness-core/tests/process.rs` (9), `docs/process-supervision.md`, `6b3a9cf` |
| 2 | File tools useful to a coding agent (search, patch, hash/range criteria, read-only recovery) | **IMPLEMENTED** | `tests/coding_loop.rs` (6), `docs/coding-loop.md`, `5dfdbfc` |
| 3 | One real model provider (OpenAI-compatible, strict, hygienic) | **IMPLEMENTED** | `crates/harness-provider/tests/provider.rs` (10), `docs/model-provider.md`, `7c122fb` |
| 4 | First autonomous coding benchmark (controlled bugs, oracle, restart) | **IMPLEMENTED** | `crates/harness-benchmark/tests/autonomous.rs` (4), `docs/coding-benchmark.md`, `02be314` |
| 5 | Durable goals and plans (DAG, repair, lifecycle, amendments) | **IMPLEMENTED** | `tests/plan.rs` (7) + plan units (3), `tests/multistep.rs` (2), `docs/durable-plans.md`, `280e68e` |
| 6 | Memory and context selection (SQLite project/episodic/procedural/failure memory) | **IMPLEMENTED** | `harness-memory/tests/memory.rs` (5), `harness-benchmark/tests/recall.rs` (3), `docs/memory.md`, `8d35740` |
| 7 | Long-running reliability (time/token/cost budgets, watchdogs, tracing) | **IMPLEMENTED** | `tests/reliability.rs` (11), provider pricing, CLI inspect test, `docs/reliability.md`, `3819cfe` |
| 8 | Expansion only after reliability (browser, delegation, GUI, self-improvement) | **PLANNED** | Gated on 1–7 |

## Capability register

| Capability | Status | Notes |
| --- | --- | --- |
| Provider-neutral Model trait, deterministic file adapter | **IMPLEMENTED** | `crates/harness-core/src/model.rs` (+ boxed-model impl) |
| Typed tools, workspace permission checks | **IMPLEMENTED** | `tools.rs`, `permissions.rs` |
| Runtime-owned file-content verification | **IMPLEMENTED** | `verification.rs` (`FileEvidenceVerifier`) |
| Hash/range criteria + `run_with_criterion` | **IMPLEMENTED** | verifier + `agent.rs` |
| SQLite events, checkpoints, budgets | **IMPLEMENTED** | `sqlite_store.rs`, 32 calls default |
| Restart recovery, conservative reconciliation | **IMPLEMENTED** | writes + read-only actions; patches/shell refused |
| Stable run-scoped action IDs | **IMPLEMENTED** | `next_action_id`, preserved across reconcile |
| 1 MiB whole-file caps; range/hash/patch/search bounds | **IMPLEMENTED** | `tools.rs`, `large_files.rs` |
| Supervised argv processes, tree kill (Windows-first) | **IMPLEMENTED** | Not isolation; Unix tree cleanup best-effort |
| OpenAI-compatible provider, CLI opt-in | **IMPLEMENTED** | Secrets header-only, never persisted |
| Scripted repair benchmark + oracle | **IMPLEMENTED** | Agent is a script, not intelligence |
| Goal/task DAG, repair, lifecycle, amendments | **IMPLEMENTED** | No inventing planner; sequential driver |
| Lexical memory (4 kinds, retention, consolidation) | **IMPLEMENTED** | `crates/harness-memory`, `docs/memory.md` |
| Reliability budgets, shutdown, heartbeats, inspection | **IMPLEMENTED** | `src/inspect.rs`, `tests/reliability.rs`, `docs/reliability.md` |
| Shell reconciliation | **BLOCKED** | Effects indistinguishable; refused by design |
| Patch reconciliation | **BLOCKED** | Before/after/conflict indistinguishable; refused by design |
| Mid-flight child-run resume | **PLANNED** | Interrupted child tasks restart from scratch |
| Memory engine, goal planner, provider router | **PLANNED** | Milestones 6–8 |
| OS sandbox / isolation | **PLANNED** | Permission checks are not a sandbox; investigate separately |

## Verification (last green, `280e68e`)

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked        # 80 passing
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run -p harness-core --example benchmark --locked        # file-core-v1 10/10
cargo run -p harness-core --example large_file_benchmark --locked  # 8 MiB verified
```

## Gap-closure round (in progress, user-directed)

| Gap | Status | Evidence |
| --- | --- | --- |
| Per-decision usage in the event log | **IMPLEMENTED** | `UsageRecorded` events, asserted in reliability tests |
| Mid-flight child-run resume | **IMPLEMENTED** | `run_task` resumes partial checkpoints; crafted + real-kill tests, exactly-once patch asserted |
| Priority-aware scheduling | **IMPLEMENTED** | `ready_tasks` orders by goal priority, unit tested |
| Unix process-group tree kill | **EXPERIMENTAL** | `setsid` + group SIGKILL via `libc`; Unix test ships but this Windows box cannot execute it — verify on Unix before relying |
| Mid-flight plan driver (Running tasks) | **IMPLEMENTED** | Adoption covers completed children; partial children re-run from scratch (documented) |

## Milestone 8 expansion (user-ordered, in progress)

| Item | Status | Evidence |
| --- | --- | --- |
| Scoped multi-agent delegation | **IMPLEMENTED** | `tests/delegation.rs` (10) + benchmark e2e (1), `docs/delegation.md`, `479ba9a` |
| Supervised network fetch + repo radar | **IMPLEMENTED** | URL gating matrix, `tests/fetch.rs` (6), CLI `--radar`, `docs/network-fetch.md`, `6093be7` |
| Controlled self-improvement | **IMPLEMENTED** | Worktree isolation, baseline/candidate gates, promote/rollback, records; `harness-experiment` (1 unit + 4 e2e), `docs/self-improvement.md`, `0295fce` |
| Browser control | **PLANNED** | Next in user order |
| GUI control | **PLANNED** | Last; heaviest platform work |

## Next up

1. Controlled self-improvement in isolated Git worktrees with benchmark
   comparison, regression gates, and rollback.
2. Keep `scripts/demo.sh` executable-mode change uncommitted and intact
   (predates implementation work; do not commit or discard it).
