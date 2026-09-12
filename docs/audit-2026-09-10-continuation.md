# Continuation: learned app operations and cancellable improvement

- Added OpenAPI 3.0/3.1 JSON discovery, persistent operation definitions, searchable
  operation details and schema-driven invocation. Required scalar parameters and
  encoding are checked before dispatch. Unsupported contracts fail explicitly.
- Saved schemas are tied to connection origins. Origin edits invalidate them;
  in-flight discovery cannot attach an old schema to a changed connection.
- Added Save, Discover, Operations, Use and Forget controls to the Apps panel.
  Management routes enforce Studio's same-origin and explicit-request headers
  and cannot dispatch arbitrary tools or invoke app operations.
- Self-improvement accepts implementation and tests together in a bounded file
  list. Every candidate file is independently read back in the isolated worktree.
- Stop now interrupts test supervision, kills the Windows process tree and rolls
  back. Failing baselines skip mutation; cancellation prevents candidate promotion.

Verification covers real local schema fetches and calls, escaped parameters,
origin invalidation, permission denials, remote/cyclic reference rejection,
unsupported contracts, real Git/Cargo multi-file experiments, cancellation of
a running 60-second test, and the complete Apps-panel browser flow. Browser
checks include untrusted schema text, actual persisted API grants, and mobile
overflow. Screenshots are in `workspace/studio-qa/connections-{desktop,mobile}.png`.

Scope and remaining limits are documented in [local app capabilities](local-apps.md).
Authentication, launcher lifecycle integration, arbitrary app compatibility and
automatic merging/reloading remain incomplete. Git operations and mutation
callbacks cannot be interrupted; Unix descendant cleanup is still limited.

Validation: 179 distinct workspace tests pass across the workspace run and a
focused fixture correction rerun. The workspace run initially reported 178
passing tests and one Windows connection-reset assertion in the new HTTP fixture;
the cross-host check now uses a bodyless request and passes. No product behavior
was relaxed. Workspace doc tests, formatting, Clippy with warnings denied,
JavaScript syntax and whitespace checks pass. Both connection-panel screenshots
were inspected. Full run evidence: `workspace/audit-2026-09-10-continuation.log`.
