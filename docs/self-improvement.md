# Controlled self-improvement

Status: **IMPLEMENTED** as an experiment harness with isolation, gates, and
rollback. Nothing here invents changes: a caller (script, or a
model) proposes exactly one mutation, and the harness decides its fate.

## What changed

- New `harness-experiment` crate. `run_experiment` executes one loop:
  1. Refuse unless the repo is a clean Git tree (dirty repos abort before
     anything is created; so do pre-existing branches).
  2. Isolate: `git worktree add -b <branch>` into a temp path.
  3. Baseline: run the check suite on the pristine repo.
  4. Mutate: apply the single candidate change in the worktree only.
  5. Candidate: run the same suite in the worktree (300 s backstop, kill
     on timeout, capped output, libtest summary parsing).
  6. Gate: failed/empty baseline or candidate, or fewer passing tests → **rollback** (force-remove the
     worktree, delete the branch); otherwise **promote** (commit on the
     branch with a harness identity via `-c` flags, remove the worktree,
     keep the branch for human merge).
- The harness never merges to the main line and never touches global Git
  config. Vacuous experiments (nothing to commit) roll back instead of
  promoting emptiness. Unparseable suite output counts as failure, never
  as success.
- Decision records (`ExperimentReport`: spec, both metric sets, decision
  with reason, timestamp) persist via `write_record` to a caller-chosen
  directory, so bookkeeping never dirties the next clean-tree check.

## Verified

- `harness-experiment` parser unit test plus `tests/self_improve.rs` (4)
  on hermetic temp Git repos: an added passing test promotes (2/0 → 3/0)
  with the branch kept, the change committed on it, the worktree gone,
  the main tree clean, and a complete record; a broken value rolls back
  with the branch deleted and the repo pristine; dirty repos and duplicate
  branches refuse without creating anything.
- Full suite: 130 passing. fmt/check/test/clippy clean.

## Limits (honest)

- Gate metric is libtest pass/fail counts only: no performance comparison,
  no flakiness detection (a flaky baseline can veto or bless wrongly), no
  coverage or lint gates.
- One mutation per experiment; no multi-step search, no automatic retry of
  near-misses, no learned proposal policy.
- Promotion stops at a kept branch: merging, review assignment, and
  main-line CI remain human responsibilities.
- Suite runs are plain subprocesses with a kill timeout, not supervised
  tool calls: they inherit the ambient environment, so hermeticity depends
  on the suite itself (fixtures pin their toolchain for this reason).

## Studio integration — 2026-09-10

Studio workers can propose 1–16 implementation and test files with self_improve
under the Terminal grant. Both suites must be nonempty and passing, and the
candidate must retain the baseline pass count. Unparseable output fails closed.
The cancellable runner checks Stop during suite supervision, stops the Windows
process tree, skips further candidate work and records rollback. Failed baselines
skip mutation too. See [local apps and capability improvement](local-apps.md).

## Studio snapshots and runtime activation — 2026-09-11

The core experiment API still requires a clean Git tree. Studio now snapshots
tracked and nonignored untracked files from dirty projects into an isolated Git
repository before invoking that API. In-progress edits stay in the original;
reports identify both repositories. A passing branch belongs to the snapshot
repository when this path is used.

Versioned skills and executable tools activate in Studio's persistent capability
registry. Compiled runtime changes can use `runtime_stage` under the supervisor,
which checks the digest/protocol and new process health and rolls back failed
startup. This does not establish semantic correctness or undo external side effects.
See the [upgrade guide](harness-upgrade-2026-09-11.md).
