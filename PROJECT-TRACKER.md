# Project Tracker — Local-first Autonomous AI Agent Harness

> Living file: update it on every feature checkpoint below. Status legend:
> **IMPLEMENTED**, **PARTIAL**, **EXPERIMENTAL**, **PLANNED**, **BLOCKED**.
> Suite **138 passing** as of 2026-09-09. Handover state: everything below
> is verified on Windows/MSVC, Rust 1.98.1.

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
| `eeced66` | Gap closure: `UsageRecorded` events, mid-flight `run_task` resume, priority scheduling, Unix process-group kill (experimental) |
| `479ba9a` | Scoped delegation: policy narrowing, budget firewall, verifier children, artifact mailbox |
| `6093be7` | Supervised network fetch (allowlist, strict URLs) + repo radar (`--radar`) |
| `0295fce` | Gated self-improvement: worktree isolation, baseline/candidate gates, promote/rollback, records |

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
| 8 | Expansion: delegation, fetch/radar, self-improvement, browser | **IMPLEMENTED** | See milestone-8 table; only GUI control remains |

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
| Supervised argv processes, tree kill | **IMPLEMENTED** | taskkill /T on Windows; group SIGKILL on Unix (**EXPERIMENTAL**, host-unverified) |
| OpenAI-compatible provider, CLI opt-in | **IMPLEMENTED** | Secrets header-only, never persisted; priced usage |
| Scripted repair benchmark + oracle | **IMPLEMENTED** | Agent is a script, not intelligence; resumes mid-flight |
| Goal/task DAG, repair, lifecycle, amendments | **IMPLEMENTED** | No inventing planner; sequential driver; priority-ordered scheduling |
| Lexical memory (4 kinds, retention, consolidation) | **IMPLEMENTED** | `crates/harness-memory`, `docs/memory.md` |
| Reliability budgets, shutdown, heartbeats, inspection | **IMPLEMENTED** | `src/inspect.rs`, `tests/reliability.rs`, `docs/reliability.md` |
| Scoped delegation + verifier children | **IMPLEMENTED** | `src/delegation.rs`, narrowing/re-grant firewall, `docs/delegation.md` |
| Supervised fetch + repo radar | **IMPLEMENTED** | Allowlist + strict URLs, no redirects/auth; fetched bytes are untrusted data |
| Gated self-improvement | **IMPLEMENTED** | `harness-experiment`; human merges, harness never does |
| Chrome control (isolated/personal/attach) | **IMPLEMENTED** | `harness-browser`; attach never kills the user browser; personal mode is explicit opt-in |
| Shell reconciliation | **BLOCKED** | Effects indistinguishable; refused by design |
| Patch reconciliation | **BLOCKED** | Before/after/conflict indistinguishable; refused by design |
| GUI control | **PLANNED** | Last remaining item; needs OS accessibility backends |
| Provider router, vector memory, goal planner | **PLANNED** | Beyond current scope |
| OS sandbox / isolation | **PLANNED** | Permission checks are not a sandbox; investigate separately |

## Verification (last green: 138 passing)

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked        # 138 passing, 0 failed
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
| Browser control (isolated/personal/attach) | **IMPLEMENTED** | `harness-browser` (8 tests), CLI `--browse-profiles/tabs/read`, `docs/browser.md`, `2c1cade`; live-verified against Chrome 150 |
| GUI control | **PLANNED** | Last remaining item; needs OS accessibility backends |

## Handover notes (read before touching anything)

1. Only remaining roadmap item: **GUI control** (Windows UI Automation
   backend). Everything else is implemented and green.
2. Environment quirks, all verified the hard way:
   - Outside this repo the default `stable` toolchain has a broken cargo.
     Fixtures pin `rust-toolchain.toml` 1.98.1; keep doing that.
   - Cleared child environments cannot link MSVC: builds need the
     documented `BUILD_ENV_GRANTS` re-granted explicitly.
   - Chrome's DevTools server rejects HTTP/1.0: speak 1.1 with
     Content-Length framing.
   - Personal browsing requires the user to start Chrome with
     `--remote-debugging-port` (their explicit consent) or to close
     Chrome for profile launch. Never kill or restart their browser.
   - Fetched web content and page DOM are **untrusted data** everywhere.
3. `scripts/demo.sh` executable-mode change predates all implementation
   work: do not commit or discard it.
4. Workflow per change: inspect → small vertical slice → `cargo fmt`,
   `check`, `test`, `clippy -D warnings` → independent verification →
   docs → update this file → recoverable commit.
