# Continuation - learned API operations and cancellable improvements

- OpenAPI discovery now saves searchable operation definitions and invokes them with validated scalar parameters. Unsupported contracts fail explicitly.
- Apps panel supports Save, Discover, Operations, Use and Forget, with persisted connections and mobile verification.
- Improvement candidates accept implementation plus tests (1-16 files). Stop interrupts suites and rolls back; failed baselines skip candidate work.
- 179 distinct tests pass across the workspace run and a corrected Windows HTTP fixture rerun. Doc tests, fmt, Clippy, JavaScript syntax and screenshot checks pass.
- Detailed evidence and limitations: docs/audit-2026-09-10-continuation.md and docs/local-apps.md.
# Local apps and improvement audit - 2026-09-10

- Added persistent app_connect, app_list and bounded loopback app_call tools in Studio, controlled by the Local APIs switch.
- Added self_improve chat actions under the Terminal grant, with isolated worktrees, fixed test gates and retained passing branches.
- Fixed experiment gates for failing, empty, unparseable and reduced-test candidates; bounded and concurrently drained suite output.
- All 170 tests report passing (24 Studio and 7 experiment tests included); fmt, Clippy with warnings denied, JavaScript syntax and whitespace checks pass. Desktop/mobile screenshots inspected.
- Evidence: workspace/audit-2026-09-10-final.log. PowerShell returned 1 due to redirected Cargo progress stderr (NativeCommandError); all 35 libtest groups report success, zero failures.
- Details: docs/audit-2026-09-10.md and docs/local-apps.md.
# Project Tracker — Local-first Autonomous AI Agent Harness

> Living file: update it on every feature checkpoint below. Status legend:
> **IMPLEMENTED**, **PARTIAL**, **EXPERIMENTAL**, **PLANNED**, **BLOCKED**.
> Suite **147 passing** as of 2026-09-09. Handover state: everything below
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

## Verification (last green: 147 passing)

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked        # 147 passing, 0 failed
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

1. GUI control remains planned. Resolve the browser lifecycle, protocol and
   delegation accounting gaps in docs/audit-2026-09-09.md before treating
   existing prototype milestones as production-ready.
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

## Audit and hardening checkpoint — 2026-09-09

- Reviewed workspace structure, docs, delegation, browser transport and fetch;
  findings and next priorities: [audit report](docs/audit-2026-09-09.md).
- Hardened child database IDs/root binding, denied shell grants to read-only
  children, bounded artifact reads, and enforced cumulative WebSocket limits
  before allocation plus a 16 KiB upgrade-header cap.
- Added six regressions. Final suite reports **144 passed, zero failed**,
  including all eight live isolated Chrome tests outside the sandbox.
- fmt/check/clippy with warnings denied pass. File benchmark: 10/10 positive
  tasks and 10/10 denied traversal cases; 8 MiB benchmark independently verified.
- The PowerShell redirected test wrapper reported NativeCommandError for Cargo's
  compilation stderr; the complete log shows all test binaries and doctests
  passed. Baseline direct test invocation also exited successfully.
- Existing scripts/demo.sh mode change preserved. GUI control remains planned.

## Visual workspace checkpoint — 2026-09-09

- Added `apps/studio` (`klyne-studio`), an embedded local web GUI connected to
  actual runtime execution and read-only SQLite inspection.
- Designed a graphite/lime control room with an execution map, live budget
  meters, event ledger, expandable evidence, run search, command palette,
  responsive layouts, keyboard shortcuts, and JSON export.
- Added isolated file-run creation, cooperative stop, conservative resume,
  persisted run discovery, bounded requests, and same-origin mutation checks.
- Added bounded Chrome viewport control for responsive verification.
- Three Studio integration tests cover API rejection, GUI execution and
  evidence, mobile overflow, and server restart without terminal replay.
- Scope: deterministic file agent only. Native Windows GUI automation remains
  planned; this checkpoint delivers the visual Klyne application.
- See [Studio documentation](docs/studio.md).

- Final validation: **147 tests passed**, zero failed; fmt/check/clippy and
  JavaScript syntax checks passed. Desktop (1440px) and mobile (390px)
  screenshots were inspected; Studio is served at http://127.0.0.1:4317.

## Chat-first Studio � 2026-09-10

- Replaced the home page with a responsive conversation UI, prompt starters,
  AI settings dialog, access switches, live worker activity, evidence and export.
- Added a Studio goal driver: model-selected sequential roles, tool execution,
  separate read-only review and up to three repair passes. Follow-up instructions
  preserve the workspace and recent context. This is separate from core delegation.
- Added transactional conversation persistence, pre-action reservations, pending
  effect refusal after interruption, cooperative Stop and shared per-turn limits.
- Web fetch and terminal switches grant real tools. Apps remain unavailable.
  Broad terminal grants are explicitly described; model review is not a formal
  success proof. Written files receive independent exact read-back checks.
- All 14 Studio tests passed, including the new chat browser and repair-loop tests.
  fmt, JavaScript syntax and Studio clippy with warnings denied passed. Desktop
  and 390px screenshots inspected in workspace/studio-qa/chat-*.png.
- Existing file-run APIs and the earlier interface remain at /files.

## Harness capability and reliability upgrade — 2026-09-11

- Configurable goal ceilings, restart recovery, saved-task resume and explicit
  uncertain-action resolution; normalized conversation history and root ownership.
- Versioned persistent skills/tools/memory; host projects and user/toolchain environment.
- Interactive CDP browser tools, authenticated HTTPS/LAN app APIs, cancellable
  model/API calls, Windows process jobs and cross-process desktop ownership.
- Dirty-project experiment snapshots; supervised binary activation and failed-startup rollback.
- Atomic persisted delegation reservations for callers of the new budgeted API.
- See [implementation and verification](docs/harness-upgrade-2026-09-11.md).

## Audit Phase 0 + Phase 1 — 2026-09-13 (security deferred per operator)

- Phase 0: char-boundary-safe history truncation (`truncate_to_char_boundary`,
  Vietnamese/emoji/mixed tests); fixed mock `Content-Length` parsing that made
  secret-hygiene assertions vacuous; test tiers documented (`docs/test-tiers.md`)
  with `scripts/test-baseline.ps1` for env-recorded runs.
- Phase 1: shared execution contract (`harness-core::execution`: `EffectState`,
  `ToolResult`, `effect_of_observation`/`effect_of_value`, one uncertainty
  classifier). All Studio dispatcher branches (browser/MCP/capability/app/
  shell/registry) now clear pending from that classifier instead of
  per-branch substring gates.
- Fixed audit HIGHs: browser post-dispatch title/text failures tagged
  uncertain (pending kept); desktop helper output joins bounded at 5s with
  uncertain expiry; `Browsers::execute` takes a typed `BrowserCall` and
  honors Stop pre-dispatch. Full cancellable browser I/O and per-session
  actors stay Phase 5 scope.
- Also collapsed 5 pre-existing clippy `collapsible_if` failures so
  `clippy -D warnings` passes on the touched crates.
- Verified: harness-core all green (incl. 5 new contract tests), Studio bin
  40 passed + 1 ignored, chat 26/26 serial, workspace check clean. Known
  pre-existing chat parallel-test isolation flake documented in test tiers.

## Audit Phase 3 (partial, broker-independent) — 2026-09-13

- Operation-bound completion receipts: `check_action_evidence` no longer
  accepts any ok evidence for effect claims. Only successful *effect*
  receipts (writes, shell, browser/desktop/app/MCP effects, host
  verification/reconciliation) confirm mutations; read-only trails
  (`read_file`, `browser_read`/`screenshot`, `desktop_observe`, listings)
  are rejected, closing the audit's "any successful evidence satisfies the
  phrase guard" hole without breaking legitimate flows (strict per-class
  binding would have rejected e.g. "created a reusable tool" backed by
  shell evidence).
- Claim triggers extended with evasion variants (`been saved/created/`,
  `successfully created/`, `was created`). Delivery claims stay
  categorically rejected (fail-closed without receipt infra).
- Verified: 4 guard unit tests, Studio bin 43 passed + 1 ignored, chat
  26/26 serial, clippy/fmt clean. Full paraphrase-proofing needs
  broker-issued receipts (deferred with Phase 2).

## Audit Phase 4 (filesystem slice) — 2026-09-13

- Atomic writes: `WriteFile` stages to a temp file and renames, so a crash
  can never leave a truncated file; no staging files leak (tested).
- Six new workspace actions: `list_dir` (sorted, <=500, truncated flag),
  `stat_path` (kind/size/readonly, symlinks refused), `make_dir`,
  `copy_file` (staged, <=64 MiB, regular files only, existing destinations
  refused — checked pre-stage so Unix rename cannot overwrite), `move_file`
  (atomic rename, dest-exists refused), `delete_path` (files and empty dirs
  only; recursive delete refused without a scoped broker).
- Permission gates: reads under `FilesystemRead` (review-safe), mutations
  under `FilesystemWrite` (revoked in review); lone `.` names the root.
- Wired through provider schema (`map_action` + prompt), Studio prompts,
  descriptors, and `Display`/`tool_name`. Reviewers inherit read-only
  directory browsing automatically.
- Verified: 4 new `file_ops` tests, provider schema tests, core lib 21,
  full core suites green, Studio bin 43 + chat 26/26 serial, clippy/fmt
  clean. Deferred: recursive search, binary streams, PTY, durable job
  manager (need broker/persistence design).

## Audit Phase 5 (browser slices) — 2026-09-13

- MCP session isolation (audit HIGH): sessions keyed per
  (server, conversation) with isolated profiles; LRU cap of 4 with child
  reaping and profile removal on evict/death; read-only connectivity probe
  in its own scope; scope strings validated for path safety.
- Correlated navigation (audit HIGH): `Page.navigate` error text evaluated,
  stale events drained, completion bound to our frame with the load event
  as freshness-checked secondary (this Chrome omits `loaderId`), landed
  URL returned in every browser observation; one shared deadline preserves
  worst-case timing. Fixed two real bugs found by testing: correlation on
  a key the event lacks, and uncorrelated waits skipping the parked queue.
- Session-mutex narrowing (audit HIGH): native browser map lock is now
  lookup/insert only; CDP I/O runs under per-session locks, so one slow
  conversation no longer blocks others.
- Verified: Tier 1 browser 8/8, Studio bin 44 + 1 ignored, chat 26/26
  serial, workspace check + clippy clean. Deferred: semantic locators,
  tabs/dialogs, downloads/uploads, vision wiring, full CDP actors.

## Audit Phase 6 (desktop slices) — 2026-09-13

- Takeover detection (audit MEDIUM): every input primitive snapshots the
  pointer and verifies it afterwards — clicks/drags must land where sent,
  keyboard input must not displace the pointer — else the effect is
  `uncertain`, never silently confirmed.
- Password-focus refusal extended from element ops to raw Type/Press via
  focused-element UIA check.
- New tools: `desktop_drag` (covered-endpoint checks, interpolated moves,
  takeover-verified landing) and `desktop_clipboard_get/set` (STA-threaded,
  4000-char caps, worker-only, wired through parse/adapter/prompts).
- Cost (audit LOW): helpers stage only on content change; PowerShell
  resolved via `%SystemRoot%` with PATH fallback instead of a fixed path.
- Verified: C# compiles, PS parses, clipboard roundtrip live, full Tier 3
  recovery script green (also proving no takeover false-positives),
  Studio bin 46 + chat 26/26 serial, clippy/fmt clean. Deferred:
  persistent helper process, richer UIA patterns, DPI qualification.

## Audit Phase 7 (planning/budgets/memory) — 2026-09-13

- Evidence-bound task completion: `Done` records `evidence_bound` from its
  own window (never blocking — answers need no tools); the reviewer gets
  per-task windows plus an instruction to prefer repairs for unbound
  completions on action goals.
- Token/cost budgets: `Connection::respond` returns metered usage (Ollama
  eval counts parsed; Codex/OpenCode meters kept instead of dropped),
  accrued per call into durable turn totals, enforced in `guard` via
  `max_tokens`/`max_cost_usd` execution limits, and visible to the model in
  context with a spend-aware planning note.
- Memory auto-recall: `recall_for_task` ranks active memories by keyword
  overlap, injects top-3 with name/version provenance into worker context,
  excludes sensitive entries from auto-injection (explicit read still
  works), caps sizes with char-boundary truncation.
- Verified: 3 new unit tests + recall tests, integration binding test,
  Studio bin 49 + chat 27/27 serial, clippy/fmt clean. Deferred:
  short-horizon replanning UX, relevance compaction, memory expiry schema.

## Audit Phase 8 (extensibility/release) — 2026-09-13

- Tool qualification lifecycle: `tool_test` binds tested/digest/timestamp
  per version (migration included); `tool_run` refuses unqualified or
  changed definitions; restore keeps per-version qualification; prompts,
  guard receipts, and the chat flow updated to save → test → run.
- MCP install separation: `mcp_setup` is the only installer (worker-only);
  reads/calls report an unavailable route with a setup directive; the
  route probe performs setup explicitly; reviewer can no longer trigger
  installs through discovery.
- Activation attestation: digest-bound, week-fresh attestations enforced
  in `preflight` for stage and supervisor paths; `klyne-supervisor
  --attest-digest/--attest-command` for operators, `runtime_attest` chat
  tool bound to file bytes; runtime suite proves reject → attest →
  activate → rollback (debugging it caught a real test-workflow gap:
  every candidate, including the failing one, needs attestation).
- Release support boundary documented in `docs/test-tiers.md` (supported
  vs excluded with the zero-unauthorized-actions release rule).
- Verified: Studio bin 51 + runtime + chat 27/27 serial, clippy/fmt clean.
  Residual: exit-zero semantics and attestation honesty still need the
  deferred Phase 2 broker.

## Audit Phase 2 (authority boundary) — 2026-09-13

- Host-owned broker (`apps/studio/src/broker.rs`, recovery-protected):
  exact-argv shell grants, name→origin secret grants (network/env
  directions separated), exact (connection, method, path) delete approvals.
  Model proposals grant nothing; user request-body grants merge only.
- Enforcement: `policy()` allowlists exact approved argv (no more
  model-named programs); saved tools need grants on top of qualification;
  app auth needs per-origin secret grants at connect and use; shell `env`
  beyond the OS minimum needs env-only grants; DELETE needs approval.
  Refusals stash exact proposals in pending; `resolve:approved` records
  the binding and the worker retries identically on resume. New
  `ApprovalNeeded` failure kind with pending-panel guidance.
- Secret hygiene centralized: granted values scrubbed before model dispatch
  (`request`) and before persistence (`chat_store::save`, incl. the action
  ledger); short/missing values skipped.
- Verified: broker unit tests, approval-loop integration (shell grant,
  exfiltration refusal with zero transmission, single-DELETE), updated
  skills/tools flow across conversations, Studio bin 58 + chat 30/30
  serial, recovery python suite green, clippy/fmt clean.
