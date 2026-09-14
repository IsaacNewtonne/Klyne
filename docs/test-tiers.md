# Test tiers — Phase 0 baseline (2026-09-13 audit)

`cargo test --workspace` is **not** one suite. Tiers separate deterministic
fixture tests from environment-gated suites so a red run means a real
regression, not a missing browser or GUI session.

## Mixed workspace baseline — offline dependencies, Chrome/GUI required

Correction from the UX/autonomy re-audit: excluding `harness-browser` does
not exclude browser use in Studio integration tests. This command launches
Chrome in `chat`, `studio`, and `site`, and exercises desktop fallback. It
is not a headless Tier 0 suite. `--offline` controls Cargo dependency access,
not the capabilities exercised by tests. Separate headless test targets are
still needed before claiming a deterministic CI tier.

```powershell
cargo test --workspace --exclude harness-browser --locked --offline --no-fail-fast
```

- Covers `harness-core`, `harness-provider` (loopback mock servers only),
  `harness-memory`, `harness-experiment`, `harness-benchmark`, `harness-cli`,
  `klyne-studio` unit + integration tests.
- `harness-core/tests/process.rs` ships two `#[ignore]` subprocess fixtures
  (`inherited_pipe_child`, `inherited_pipe_parent`); they run only as helpers
  of `deadline_covers_pipes_retained_after_parent_exit`, never standalone.
- Provider tests use a std-only loopback mock — no vendor endpoint, no TLS to
  the public internet. Secret-hygiene assertions verify the captured request
  body, which requires the mock to parse `Content-Length` case-insensitively
  (hyper sends it lowercase).

## Tier 1 — browser (requires Chrome)

```powershell
cargo test -p harness-browser --test controlled --locked --offline -- --test-threads=1
```

- 8 tests against a local fixture page plus isolated Chrome. Serial threads:
  one Chrome instance at a time.
- Fails closed with "headless Chrome must launch" when Chrome is absent —
  that is an environment skip, not a Tier 0 regression.

## Tier 2 — live MCP (ignored by default, requires npm + fixture root)

```powershell
$env:KLYNE_MCP_TEST_ROOT = (Resolve-Path .).Path
cargo test -p klyne-studio --bin klyne-studio --locked --offline -- --ignored mcp::tests::live_browser_roundtrip
```

- `mcp::tests::live_browser_roundtrip` launches the installed browser MCP
  against a disposable loopback page. Never runs in Tier 0.

## Tier 3 — desktop (Windows GUI session required)

```powershell
powershell -NoProfile -File scripts/test_desktop_recovery.ps1
```

- Drives a real WPF fixture window (focus/minimize/restore, field fill,
  truncation flags, stale-layout rejection, modal handling). Cannot run
  headless or in CI sandboxes.

## Tier 4 — live providers (no automated suite)

- No test dials a real model endpoint. `docs/model-provider.md` records the
  mock-verified contract; vendor behavior is explicitly out of scope.

## Parallel-test isolation (fixed 2026-09-13)

- `restart_reconciles_...` failed under default parallel threads with
  `cannot access index 0 of JSON array of length 0`. Root cause, found by
  tracing save history: the turn never planned (only send-time + terminal
  saves existed) because `send()` failed the desktop lease instantly while
  another test server held the system-wide lock file — Blocked with zero
  tasks, then the test indexed `tasks[0]`.
- Fixes: `send()` now waits boundedly (30s) for the desktop lease, matching
  its own documented behavior, instead of failing a second conversation on
  a transient holder; the chat harness verifies its child actually won the
  port bind (spawn retry) and rebinds verified on respawn; the test asserts
  planned tasks with a clear message instead of indexing blindly.
- Chat suite is now green in default parallel mode (3 consecutive runs);
  `-- --test-threads=1` remains the fallback for constrained machines.

## Phase 1 conformance notes (2026-09-13)

- `harness-core/src/execution.rs` unit tests cover the shared classifier:
  uncertainty markers (all supervised adapters), plain refusals as
  `NotApplied`, success-by-mutation mapping, and every JSON branch shape
  (browser success shapes without `ok`, MCP/capability/app flags, desktop
  `known_not_applied`).
- `desktop::tests::output_collection_is_bounded_and_marks_expiry_uncertain`
  proves bounded joins and that expiry classifies as `Unknown`.
- Browser post-dispatch read-back conformance (navigate/click/fill then failed
  title/text keeps pending) is Tier 1: it needs isolated Chrome and is
  exercised by `harness-browser --test controlled` plus manual Studio runs,
  not Tier 0.

## Phase 3 conformance notes (2026-09-13)

- `completion_guard` unit tests prove operation-bound receipts: read-only
  trails (`read_file`, `browser_read`/`screenshot`, `desktop_observe`) can no
  longer confirm mutation claims; effect receipts (writes, shell, browser/
  desktop/app/MCP effects, host verification) do; paraphrase variants
  (`been saved`, `created successfully`, `was created`) still trigger.
- Full chat suite (26/26 serial) passes under the tightened guard, so no
  legitimate completion flow relied on observational evidence.
- Residual: paraphrase outside the claim list still evades; categorical
  delivery rejection stays fail-closed until broker-issued receipts exist
  (deferred Phase 2).

## Phase 4 conformance notes (2026-09-13)

- `harness-core/tests/file_ops.rs` (4 tests): atomic write overwrite with no
  staging leaks, list/stat shapes, copy/move no-overwrite refusals (dest
  content preserved), delete refusing non-empty dirs and missing paths,
  traversal denial for all six new ops, and review parity (write revoked →
  list/stat allowed, mkdir/copy/move/delete denied).
- `harness-provider` directory-tool schema tests: all six new tools parse to
  typed actions; missing fields and unknown tools stay `Fail`.
- `permissions.rs`: lone `.` names the workspace root; trailing-dot/space
  filenames stay denied.
- Deferred remainder: recursive search, binary transfer streams, PTY/
  interactive terminal, and the durable job manager (all need the scoped
  authority broker or a persistence design from Phase 2/3-full).

## Phase 5 conformance notes (2026-09-13)

- Tier 1 (`harness-browser --test controlled`, 8/8): navigation now
  evaluates `Page.navigate` error text, drains stale events, binds
  completion to our frame (`frameStoppedLoading[frameId]`), and returns the
  landed `document.URL` in every browser observation. Debugging note: this
  Chrome emits `loadEventFired` with timestamp only (no `loaderId`), so
  frame correlation is primary and the drained load event is secondary.
- MCP isolation is covered without a browser: scope-validation unit tests
  plus the existing grant tests. The live MCP roundtrip (`--ignored`, Tier 2)
  now runs in a `live-test` scope.
- Deferred remainder: semantic locators, tabs/frames/dialog tools,
  download/upload contracts, vision wiring, full per-session CDP actors
  (native sessions still share one CDP socket each; the map mutex is now
  lookup-only so slow sessions no longer block others).

## Phase 6 conformance notes (2026-09-13)

- Rust unit tests: drag endpoint bounds, clipboard 4000-char cap and
  worker-only gating, helper staging skip-if-unchanged.
- C# compiles (`Add-Type`); both PS scripts parse; clipboard roundtrip +
  cap verified live; **full Tier 3 `test_desktop_recovery.ps1` passes on
  this box**, which also proves the new takeover checks don't
  false-positive on real focus/fill/invoke flows.
- Takeover detection itself (pointer displacement vs agent intent) is
  fail-closed by construction but has no automated positive test —
  synthesizing real user input mid-operation is flaky by nature; manual
  verification is through the emergency corner/Pause path.
- Deferred remainder: persistent supervised helper process (per-call
  PowerShell spawn + C# compile remain; staging now skips unchanged
  rewrites), richer UIA patterns, hover, window close, multi-monitor/DPI
  qualification.

## Phase 7 conformance notes (2026-09-13)
- Evidence-bound completion: `task_completion_records_evidence_binding_for_review`
  (write-backed Done bound, tool-free Done unbound) plus unit tests for
  window computation and reviewer window payloads.
- Budgets: unit tests prove token (prompt+completion) and spend guards trip
  before dispatch; invalid-usage accounting covers failed-validation calls
  too. No live spend test — providers report real meters only against real
  endpoints (Tier 4, none automated).
- Memory recall: ranking, provenance, secret exclusion, and empty-store
  cases unit-tested against a temp capability DB.
- Deferred remainder: short-horizon replanning UX, relevance-ranked context
  compaction beyond the 90k eviction, memory expiry/confidence schema.

## Recording the environment

Run `scripts/test-baseline.ps1` — it writes
`workspace/test-baseline-<timestamp>.log` with `rustc -V`, `cargo -V`,
active toolchain, OS build, Chrome version (if present), and the mixed baseline
result. Attach that log to any baseline claim.

## Phase 8 conformance notes (2026-09-13)

- Tool qualification: version-specific tested/digest/timestamp binding with
  migration for pre-existing databases; `tool_run` refuses unqualified
  definitions; restore semantics covered; chat flow updated to
  save → test → run.
- MCP install separation: missing-package reads/calls report an
  unavailable route with a `run mcp_setup` directive; no npm side effects
  without the explicit tool; reviewer/setup gating covered.
- Activation attestation: digest-bound, command-recorded, 7-day-fresh
  attestations enforced in `preflight` (both stage and supervisor paths);
  supervisor CLI gains `--attest-digest/--attest-command`; chat gains
  `runtime_attest`; the runtime suite proves reject → attest → activate →
  rollback.

## Release support boundary (2026-09-13)

What these tiers do and do not establish, stated plainly for release claims:

- Supported: workspace-scoped file operations (12 tools), supervised argv
  execution with tree kill, loopback mock-verified model contracts (no
  vendor behavior claimed), isolated Chrome automation against controlled
  fixtures, per-conversation MCP browser sessions, Windows desktop control
  on an interactive session, SQLite durability with conservative recovery,
  staged upgrades with attested rollback.
- Explicitly excluded: cross-vendor model task-success rates, arbitrary
  websites and authenticated services, arbitrary desktop applications,
  unattended installations, message delivery, privileged/elevated changes,
  sandboxing of supervised processes (Job Objects manage lifetime, not
  security), and semantic proof of tool outputs (exit zero records a run;
  meaning stays reviewer-verified).
- Release rule from the audit: zero unauthorized or duplicate consequential
  actions in the release corpus, with exclusions and skipped live tests
  reported — not silently passed.

## Phase 2 conformance notes (2026-09-13, historical)

- Broker unit tests: exact-argv shell binding, name→origin secret binding
  (lookalikes refused, network/env directions separated), delete triples,
  grant-shape validation, merge-only semantics, scrub coverage.
- Adversarial integration: ungranted shell pauses with zero execution;
  attacker-origin secret connect transmits nothing; DELETE executes exactly
  once post-approval with the grant recorded; altered argv/origin/path
  variants covered at unit level.
- Recovery guardian refuses `broker.rs` edits (python suite green).
- Residual: desktop/API consequential actions beyond shell and DELETE have
  no host-classifiable approval gate; improvement flows keep their own
  gates; attestation honesty and exit-zero semantics still trust the
  operator/model loop.

The [September 14 implementation notes](autonomy-implementation-2026-09-14.md)
supersede the DELETE and attestation contracts above: DELETE now binds resolved
origin/query/body as well, and attestation requires a host-executed successful
test process. Semantic correctness still requires appropriate verification.
