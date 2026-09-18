# Repository completeness audit — September 17, 2026

## Scope

Inventoried source, tests, scripts, documentation and deployment configuration
across all eight Rust workspace members, Studio web assets and the public demo.
Reviewed recovery, model dispatch, persistence, tool provenance and UI integration
in detail. Ran the workspace suites, JavaScript syntax checks, activity UI checks
and Python recovery tests. This is not a claim of line-by-line formal verification
or proof of arbitrary unattended task success.

## Findings and fixes

1. **Retry feedback could bypass prompt redaction.** Scrubbing previously ran
   before entering the retry loop. It now runs immediately before every model
   request, after adding new diagnostics and correction feedback.
2. **Recovery incidents bypassed database redaction.** The separate incident
   writer received raw errors and model responses. Incident payloads now pass
   through the granted-secret scrubber. Responses and diagnostic errors are
   scrubbed before truncation, so a partial credential cannot escape matching.
   An integration test covers a synthetic credential crossing the length limit.
3. **Old diagnostics contaminated new goals.** A persisted goal-start event ID
   now limits active diagnostic context. Starting a new goal preserves audit
   history; resuming the existing goal preserves its current findings.
4. **Retry diagnostics were stale.** Each retry refreshes its diagnostic context
   from the journal, including the failure which triggered the correction.
5. **Historical reads could masquerade as fresh blocker evidence.** A new
   `evidence_read` record could cite an old observation as if newly inspected.
   Blocker checks now reject historical wrappers and null observation payloads.
6. **UI tests and demo text lagged behind the activity redesign.** Updated
   assertions cover model waiting, tool activity versus historical evidence,
   idle composer borders, explicit conversation toggles, on-demand inspectors,
   the Output panel and the demo reactor. No tests are removed or ignored to
   hide these mismatches. Existing animation, reduced-motion, mobile, completion
   and user-interaction assertions remain.
7. **Documentation and generated assets needed reconciliation.** Corrected the
   GPU scope description, removed the obsolete demo ember instruction, rebuilt
   shared assets, and cleaned whitespace errors.
8. **A Windows fixture socket was timing-dependent.** The DELETE approval test
   accepted a nonblocking socket and immediately read a complete request. It
   intermittently failed with OS error 10035. The accepted socket now uses
   blocking reads with its existing timeout; the no-replay assertions remain.

## Reproduction

Run from the repository root in a Windows GUI session with Chrome available.
The separate target directory avoids replacing the running supervisor binary.

```powershell
cargo test --workspace --locked --offline --target-dir target/autonomy-upgrade --no-fail-fast -- --test-threads=1
node scripts/test-activity-all.cjs
python -m unittest discover -s scripts -p test_klyne_recovery.py
git diff --check
```

`--offline` requires dependencies already cached. Browser/desktop tests need
execution outside the restricted sandbox. The JavaScript browser checks use
the locally installed Playwright package under `workspace/studio/conversations/
mcp-packages/node_modules`; a fresh checkout needs that dependency installed.

## Validation results

- **324 Rust tests covered and passing after corrections.** The final workspace
  run passed 323 and exposed the Windows fixture race above. After fixing only
  that test fixture, the entire 43-test conversation target passed. No UI tests
  were filtered out. Ten ignored entries are subprocess helpers and the live-MCP
  test; they are not counted as passing tests.
- Three JavaScript suites passed: welcome effects, activity state reducer and
  activity UI (mobile, reduced motion, waiting, independent completion/failure).
- Python recovery suite: 13 passed, one environment-dependent skip.
- Syntax checks passed for all 13 JavaScript/CommonJS files in the checked
  application, site and scripts directories. Shared generated assets match.
- `git diff --check` passed.
- Workspace output: `workspace/audit-completeness-final-workspace.log`.
  Corrected conversation target: `workspace/audit-completeness-final-chat.log`.
- Candidate SHA-256:
  `588511f816bde4177048a4a13b3f33e35fa614b00be10f1c9f4ced3b710ece01`.
  Staging reruns the four pure diagnostic unit tests against the same bytes.
- Activation confirmed: `current.json` and `last-result.json` select that
  candidate, `rollback:false`; runtime PID 57944 reports `ready:true`.

## Remaining capability gaps

- Live Chrome profile selection and WhatsApp delivery have not been validated
  by these fixtures. No real message was sent during this audit.
- No measured live-provider task corpus yet establishes completion rates,
  intervention rates or reliability gains across tools.
- Blocker semantics still depend on model interpretation. Structural provenance
  checks cannot prove that an observation supports every natural-language claim.
- The diagnostic journal is bounded and records decision corrections; it is not
  a full AgentRx semantic judge or an immutable, complete execution transcript.
- New adapters still need schemas and operation-specific outcome verification.
  Diagnostic history does not automatically learn or approve executable repairs.
- Live MCP and dedicated Windows desktop fixture suites are separate tiers;
  ordinary workspace tests do not establish their live deployment behavior.
- The existing GitHub workflow publishes the demo; it does not run the complete
  Windows/GUI regression gate. This audit ran that gate locally.

These are explicit limits and future engineering work, not hidden TODO stubs.
