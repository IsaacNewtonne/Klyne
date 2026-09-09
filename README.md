# Rust Agent Harness

A local-first Rust runtime prototype. **MODEL != AGENT**: models propose actions; the runtime owns permissions, evidence, budgets, and persistence.

## IMPLEMENTED and tested

- Two-crate workspace: harness-core and harness-cli.
- Replaceable model trait and deterministic file-objective adapter.
- Typed file actions, permission checks, independent runtime read-back verification.
- Serializable success criteria and a verifier trait independent of the model adapter.
- SQLite migrations, versioned events, transactional checkpoints, process-exclusion lock.
- Durable objective, history, step budget, pending action, and terminal outcome.
- Resume between actions after process termination; refuse uncertain interrupted actions.
- Explicit `--reconcile` inspects interrupted file actions without replaying writes; action audit records carry stable run-scoped IDs.
- Persisted tool-call reservations (32 by default), including verification and reconciliation; filesystem reads/writes capped at 1 MiB per operation.
- Typed range reads, streaming SHA-256, and digest-guarded patches for larger files; see [large-file operations](docs/large-files.md).
- Structured tool descriptors and discovery (`--tools`); supervised argv process execution with explicit grants, timeout, output bounds, and Windows tree kill; see [process supervision](docs/process-supervision.md).
- Bounded single-file substring search plus hash/range-based success criteria; a deterministic scripted agent can inspect, patch, and verify a controlled file through the runtime (`run_with_criterion`), with read-only recovery and blocked patch reconciliation; see [coding loop](docs/coding-loop.md).
- First real model provider: OpenAI-compatible chat-completions adapter with strict validated decisions, credential hygiene, timeouts, response caps, bounded retries, and usage accounting, plus CLI opt-in via `--openai-compat`; see [model provider](docs/model-provider.md).
- First autonomous coding benchmark: scripted repair of controlled Rust bugs through inspect → patch → `cargo test` → failure-driven repair → restart, with an independent compiler/test oracle and metric reports; see [coding benchmark](docs/coding-benchmark.md).
- Durable goals and plans: persisted goal/task graphs with dependencies, blocked/abandoned states, evidence-driven repair, lifecycle states, and recorded budget amendments; multi-task plans survive kills without duplicating tasks; see [durable plans](docs/durable-plans.md).
- Memory and context selection: SQLite-backed project/episodic/procedural/failure memory with confidence, provenance, explained retrieval, retention, and consolidation; recalled experience improves repeated benchmarks without blind replay; see [memory](docs/memory.md).
- Long-running reliability: wall-clock/token/cost budgets, repeated-error detection, graceful shutdown, step heartbeats, and machine-readable run inspection (`--inspect`); see [reliability](docs/reliability.md).
- Scoped delegation: fenced child runs with narrowed scope, inherited limits, budget firewall, read-only verifiers, and artifact collection; see [delegation](docs/delegation.md).
- Negative security tests, actual process-kill recovery test, checkpoint-boundary tests, supervised-process tests (nonzero exit, timeout, output bounds, denials, env grants, tree kill).

## Run

Requires pinned Rust 1.98.1 and a native C compiler for bundled SQLite.

```sh
cargo run -p harness-cli -- --workspace workspace/example --objective "create file hello.txt with content hello agent"
cargo run -p harness-cli -- --workspace workspace/example --resume
cargo run -p harness-cli -- --workspace workspace/example --reconcile
cargo run -p harness-cli -- --workspace workspace/example --events
cargo run -p harness-cli -- --workspace workspace/example --tools
```

One run per database, defaulting to `<workspace>/.harness/run.sqlite3`. Use a fresh workspace or explicit `--database <path>` for another run. Terminal resume returns the historical result without new actions or fresh verification. The legacy text store remains available to library callers without recovery support.

Ordinary `--resume` refuses pending actions. `--reconcile` rechecks permissions and reads the target: matching interrupted writes are recorded as satisfied postconditions, pending reads receive fresh observations, and normal execution resumes. Missing or conflicting contents remain pending; no corrective write is attempted. Shell actions cannot be reconciled. Matching contents prove current state, not that the original process wrote them.

The adapter accepts `create file <relative-path> with content <text>` or `write file <relative-path> :: <text>`. It is not an LLM and cannot solve general coding tasks. Hash/range criteria and `run_with_criterion` are available to typed model implementations through the library interface, not the CLI grammar.

## Verify and benchmark

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run -p harness-core --example benchmark --locked
cargo run -p harness-core --example large_file_benchmark --locked
```

The benchmark reports JSON and fails if any of 10 file tasks or 10 traversal cases violate acceptance criteria. It has no external model calls, tokens, or model cost.

## PARTIAL / limitations

- Path checks reject traversal, reserved runtime paths, existing symlinks and Windows reparse points. They are **not an OS sandbox**: concurrent path replacement, hard links and hostile workspace mutation remain unresolved. Use trusted isolated workspaces. Writes are not atomic.
- Shell execution defaults to denied and requires explicit per-executable (optionally argument-prefix) grants. Supervised runs use argv only, a workspace-fixed cwd, a cleared environment with explicit grants, a 30 s timeout, 64 KiB per-stream output caps, and Windows `taskkill /T` tree cleanup. This is authorization, not OS isolation: no job objects, cgroups, or sandbox exist yet, and Unix tree cleanup is best-effort.
- Checkpoints are authoritative; events and checkpoints are not one transaction across an entire tool call. Reconciliation audit attempts may repeat after a crash; consumers should group them by action ID. IDs are scoped to a run/database, not globally unique idempotency keys. No exactly-once or host-power-loss guarantee.
- Recovery assumes a stateless model. One database handles one run. Cognitive-step and tool-call budgets exist; time, token and monetary budgets remain planned.
- Objectives, observations and checkpoints contain task data. Secret redaction is absent: do not supply secrets.
- General planners, model routing, general coding intelligence, vector memory, browser, GUI, and self-improvement are **PLANNED**. Provider, scripted repair benchmark, durable plans, lexical memory, reliability budgets, and scoped single-level delegation are implemented; OS isolation is investigated but not built.

See [implementation evidence](docs/milestone-durable-core.md) and [historical architecture](docs/architecture-v0.1.md).

New objectives accept `--max-tool-calls <integer>` (default 32). This cannot override a resumed run's persisted budget. Reservations commit before invocation; failed reads count, and a crash can consume credit without executing a call. Budget exhaustion returns an error and leaves the checkpoint for inspection. It does not mark the goal complete. Active legacy checkpoints without accounting refuse continuation because prior reconciliation usage cannot be reconstructed reliably; terminal legacy results remain readable. No automatic budget replenishment or migration is implemented.

Whole-file text reads accept only regular UTF-8 files and consume at most 1 MiB plus one detection byte. Oversized input is rejected without truncated success. Whole-file writes retain the 1 MiB limit. Range reads select up to 1 MiB; streaming hashes and targeted patches operate on files up to 64 MiB. These limits do not bound model output, total history, checkpoint size or elapsed I/O time.
