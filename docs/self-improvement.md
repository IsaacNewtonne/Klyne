# Controlled self-improvement

Status: **IMPLEMENTED** as an experiment harness with isolation, gates, and
rollback. Nothing here invents changes: a caller (script, and eventually a
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
  6. Gate: more failures than baseline → **rollback** (force-remove the
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
