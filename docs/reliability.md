# Long-running reliability

Status: **IMPLEMENTED** for budgets, failure detection, shutdown, heartbeats,
and inspection. OS isolation remains **PLANNED** (investigated, not built;
see below).

## What changed

- Cumulative resource budgets, all persisted and resume-stable:
  - Wall-clock origin (`started_at_ms`) is set once at creation and never
    reset by resume; exceeding `wall_clock_secs` fails closed with
    `WallClockExhausted`. Legacy checkpoints backfill the clock on first
    drive (documented fresh clock, not reconstructed history).
  - Token and cost spend accrues from `Model::usage()` deltas after every
    decision (high-water mark; providers report cumulative spend) and
    persists as `used_tokens`/`used_cost_usd`. Limits fail closed with
    `TokenExhausted`/`CostExhausted`. Unmetered models report zero and are
    unaffected. Provider pricing (`Pricing`, per-1k rates) turns token
    counts into cost; without it cost stays zero.
  - `amend_resource_limits` replaces the whole set with an audited reason
    (`ResourceLimitsAmended`); like tool-call amendments, nothing resets
    silently.
- Repeated-error detection: trailing failed observations are counted every
  step; `max_consecutive_failures` (default 3, 0 disables) fails the run
  with `RepeatedFailures`. Isolated failures interleaved with success never
  trip. Permission denials settle runs immediately and do not accumulate.
- Graceful shutdown: a process-local `ShutdownHandle` (cloned before
  driving) checkpoints progress and stops the loop with a resumable error;
  no outcome settles. Resume continues where it stopped.
- Heartbeats every 10 cognitive steps carry the live budget snapshot
  (`Heartbeat` events) for watchdogs and inspection.
- Machine-readable inspection: `inspect::inspect_run` returns one JSON
  snapshot (objective, lifecycle, steps, tools used/limit, tokens, cost,
  limits, history length, pending action, outcome, goals/tasks rollup,
  amendments, event-kind histogram); CLI `--inspect` prints it.
- Natural completion now marks lifecycle `Completed` (failed runs keep the
  plan active for inspection and repair).

## Verified

`crates/harness-core/tests/reliability.rs` (11): wall-clock trip with zero
actions and surviving origin; token/cost accrual with exact recorded spend;
3-consecutive trip with no side effects vs interleaved success completing;
shutdown stopping without settling then resuming to completion; heartbeat
snapshot contents; a 202-action run completing with 20 steady heartbeats;
5 repeated resumes repeating nothing; inspect JSON shape. Plus a provider
pricing test (2000/1000 tokens at $0.01/$0.03 → $0.05) and a CLI `--inspect`
test. Full suite: 101 passing. fmt/check/test/clippy clean.

## Limits (honest)

- No OS isolation. Investigated, not built: permission checks are lexical
  path/capability gates, not a sandbox. Concurrent path replacement, hard
  links, and hostile workspace mutation stay out of the threat model;
  `cargo` children (e.g. `rustc`) inherit whatever the OS gives them.
  Real isolation needs OS primitives (Windows job objects + restricted
  tokens, Linux namespaces/cgroups/seccomp, macOS sandbox profiles) plus a
  broker for file access — a dedicated milestone, not a flag.
- Shutdown is cooperative between steps, not preemptive: a hung tool call
  still relies on its own timeout (process supervisor) to return.
- Token/cost accounting trusts provider-reported usage; hostile providers
  can under-report. No independent token counter exists.
- Usage deltas persist at the next checkpoint, not synchronously with the
  decision; a crash in between under-counts rather than over-counts.
- Heartbeats are step-based, not time-based; a stuck step produces no
  heartbeat by construction.
