# Scoped multi-agent delegation

Status: **IMPLEMENTED** for single-level parent/child delegation with
inherited limits. Multi-level chains, parallel children, and cross-task
budget pooling remain **PLANNED**.

## What changed

- Delegation is a contract (`ChildGrant`), not ambient authority. `spawn_child`
  enforces four properties before and during the run:
  1. Narrowed scope: the child workspace is a subdirectory validated by the
     parent's own path checks; escapes, absolute paths, reserved paths, and
     existing non-directories are rejected before spawning.
  2. Inherited limits: the child keeps file capabilities minus shell; every
     shell/env re-grant is verified against a covering parent grant
     (`grant_shell_from`, `grant_env_from`). Privilege shrinks down the
     chain, never grows. Verifier mode additionally revokes write access.
  3. Delegated budget: the child's tool-call ceiling must fit inside the
     parent's remaining budget, checked before the child database is even
     created. Over-delegation fails closed with no side effects.
  4. Collected evidence: the child runs in its own database through the
     full boundaries (permissions, budgets, verification); the parent
     re-reads artifacts itself (`collect_artifacts`, bounded by count and
     bytes, non-recursive so build trees stay out) and records outcomes.
- `narrow_to_subdir`, `grant_shell_from`, `grant_env_from`,
  `revoke_capability`, `capabilities`, and `allows_shell_program` on
  `PermissionPolicy` are the primitives; the cleared-by-default slate is
  what makes non-inheritance structural rather than conventional.
- Shared project memory across the boundary is a provenance-tagged store
  file both sides open; children recall it like any other model input, and
  verification still runs after recall.

## Verified

- `harness-core/tests/delegation.rs` (10): in-scope work with the parent
  policy untouched; traversal escape failing closed; scope rejection before
  spawn; shell denied without re-grant and executed with one; escalation
  beyond parent grants refused (unknown program, narrower-than-parent
  prefix); read-only verifier denied on write with reads intact; budget
  firewall blocking over-delegation without creating state and accounting
  exact fits; artifact bounds; env re-grant reaching child processes and
  absence otherwise (platform-gated); delegation recorded into a parent
  plan checkpoint with artifact evidence.
- `harness-benchmark/tests/delegation.rs` (1): a worker child repairs a
  fixture through narrowed scope + re-granted cargo/test/build env, then a
  read-only verifier child recalls the expected fix from shared project
  memory, reads the file back, and completes under independent runtime
  verification. Its event log contains zero file-mutating tool calls.
- Full suite: 116 passing. fmt/check/test/clippy clean.

## Limits (honest)

- Single level only: a child cannot itself delegate (no API forbids it yet,
  but budgets/scopes were not tested for chains).
- Sequential children; no parallel spawning, no shared budget pool, no
  deadline propagation into children.
- The verifier pattern trusts the model to actually check; a lying verifier
  is caught only by the runtime's independent verification read, not by
  deeper proof. Verifier agents that need compile/test evidence should run
  the oracle pattern from the benchmark instead.
- Artifact collection is non-recursive and UTF-8 lossy; binary artifacts
  are out of scope.
