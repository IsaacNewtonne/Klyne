# AgentRx-inspired recovery diagnostics

Follow-up fixes and repository-wide verification are recorded in the
[September 17 completeness audit](audit-completeness-2026-09-17.md).
Diagnostic context is now scoped to the current goal and refreshed on retries;
retry prompts and separate incident files share granted-secret redaction.

Source: [AgentRx v2, August 31, 2026](https://arxiv.org/html/2602.02475v2)
and the [authors' implementation](https://github.com/microsoft/AgentRx).

The paper combines normalized trajectories, guarded constraints and evidence
logs to diagnose unrecovered failures. Its limitations include false positives
and confusing downstream symptoms with causes. Diagnosis is not a guarantee
of successful autonomous recovery.

## Implemented in Klyne

- A host-owned, persisted decision-check journal in `execution.diagnostics`.
  Entries identify the request episode, role, monotonic event ID, check, and
  number of tool observations available at that point. Existing tool history
  supplies the corresponding observations by index.
- Decision-schema, model-transport, unsupported-blocker and browser-inspection
  corrections are recorded before retrying. Accepted replacements resolve only
  findings from their own request episode. `decision_corrected` is explicitly
  distinct from verified tool or task success.
- The model receives the latest twelve checks to guide subsequent diagnosis.
  The journal retains at most 64 events and 512 characters per finding; it is a
  bounded recovery history, not a complete raw model transcript. Normal chat
  persistence and its granted-secret scrubber apply.
- Environment blockers cannot cite observations from before the current goal.
  This checks provenance, not whether the content semantically proves a claim.
- Built-in file, fetch and shell proposals are parsed before dispatch. Malformed
  requests enter the existing bounded model-correction loop without running a
  tool. Other adapters keep their existing schema validation at dispatch.
- All tools, including future adapters, benefit from shared blocker checks and
  model-decision diagnostics. New tool schemas still need adapter validation.

The implementation does not execute generated checker code, make diagnostics
into permissions, retry uncertain effects, or turn a model's success statement
into a receipt. Existing stops, budgets, approvals and effect reconciliation
remain authoritative. No automatic cross-conversation learning is added here.

## Verification

Unit cases cover current-goal provenance, bounded serialization, and resolving
one episode without hiding an earlier unresolved episode. An execution fixture
returns a malformed file request, corrects it to directory inspection, completes,
and checks the saved diagnosis and absence of a dispatched malformed request.
Repeated unsupported questions retain open findings and never claim success.

This is a focused adaptation, not a reproduction of the full AgentRx judge or
its benchmark. Semantic diagnosis, verified cross-run recovery strategies and
live desktop outcome evaluation remain further work.

### September 16, 2026 regression result

135 checks passed: 88 Studio unit, 40 execution integration, 5 core security,
and 2 supervisor tests. Four subprocess/live-MCP fixtures were ignored.
The two previously failing visual UI tests listed in `tool-autonomy-upgrade.md`
remain excluded. Browser fixtures require execution outside the sandbox.
The Stop fixture now synchronizes on actual model-request receipt, fixing its
race with cancellation before dispatch. The persistence test also covers
redaction of known secrets in diagnostic findings.

Candidate SHA-256:
`0360edaa7d4d50d055c329903da224a6913446e4c3fe4ab639f2dbb9ab40b438`.
After the regression gate, staging reran the three pure diagnostic unit tests
against that candidate. This run did not test live WhatsApp delivery or measure
an autonomy improvement rate with the configured Ollama model.
