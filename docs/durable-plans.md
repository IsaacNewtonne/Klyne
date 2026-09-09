# Durable goals and plans

Status: **IMPLEMENTED** as persisted plan state with explicit transitions and
a hierarchical driver. A general planner (something that *invents* the graph)
remains **PLANNED**; the runtime durable keeps whatever graph it is given.

## What changed

- `harness-core::plan`: goals (id, description, priority) plus tasks with
  explicit dependencies, statuses
  (`pending → running → succeeded`, plus `failed`, `blocked`, `abandoned`
  with reasons), evidence notes, and supersede links. Registration validates
  duplicates, unknown goals/deps, self-deps, and cycles; violations are
  rejected, never stored. `ready_tasks()` schedules only pending tasks whose
  deps all succeeded, ordered by goal priority then registration order.
  deps all succeeded. `repair()` abandons a failed/blocked task with its
  reason preserved and registers a replacement inheriting its dependencies;
  the goal and every attempt stay in the record.
- `RunState.plan` persists all of this in the existing SQLite checkpoint
  (serde-defaulted, so legacy checkpoints load as an empty active plan).
- Runtime plan APIs, each committing before returning:
  `create_run` (seed without driving, for external schedulers),
  `run_with_plan` (seed and drive),
  `revise_plan` (durable transition + `PlanRevised` audit; terminal runs
  refuse), `plan_snapshot` (durable read), `set_lifecycle`
  (`Active`/`Paused`/`Cancelled`/`Completed` + `LifecycleChanged`),
  `complete_run` (refuses unless every task is succeeded or abandoned, then
  terminal `Completed`), `cancel_run` (terminal `Failed` with the reason),
  `amend_tool_budget` (records previous limit, new limit, and reason in the
  plan plus a `BudgetAmended` event; limits are never silently reset).
- Driving honors lifecycle: paused runs error without settling an outcome,
  cancelled runs fail closed, completed runs refuse further steps.
- Hierarchical scheduling without duplicating effects: the benchmark driver
  (`run_plan`) keeps the parent plan in one database and runs each task as a
  full child run (permissions, budgets, verification) in its own database.
  Resume adopts terminal child completions re-confirmed by the oracle, and
  re-runs only what never completed. The kill window between child
  completion and success recording is closed by adoption too.

## Verified

- `harness-core` plan unit tests (3): dependency ordering, duplicate/unknown/
  self-dep/cycle rejection, repair preserving goal and attempts.
- `harness-core/tests/plan.rs` (7): restart with goal and statuses intact,
  terminal refusal, failure → repair → completion, recorded amendments,
  pause blocking drive without settling then resuming to completion,
  terminal cancel surviving restart, completion gated on finished tasks.
- `harness-benchmark/tests/multistep.rs` (2): a two-task cargo repair plan
  survives a real process kill after task one; resume adopts it (child event
  log byte-identical) and completes task two with the top-level goal intact.
- Full suite: 80 passing (68 prior + 12 plan). `cargo fmt --check`,
  `cargo check --locked`, `cargo test --locked`, `cargo clippy -- -D warnings`.

## Limits (honest)

- No planner: graphs are built by callers or scripts, not invented from
  objectives. Priorities are recorded but not yet scheduled by.
- The driver is single-threaded and sequential; no parallel tasks, no
  deadline propagation, no cross-task budget pooling.
- A child run interrupted mid-flight (no terminal outcome) is re-run from
  scratch; resuming a partial child run is planned.
- `Blocked` tasks need operator repair; nothing auto-unblocks.
- Provider usage and wall-clock budgets from milestone 3 are still
  client-side; amendments cover tool-call limits only.
