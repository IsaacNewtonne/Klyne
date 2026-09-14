# Klyne UX and autonomy re-audit — 2026-09-13

Base: `a02895a`. Scope: local working tree, all eight workspace members,
Studio presentation, execution routes, authority, persistence, recovery,
extensions, provider orchestration, release scripts and existing tests.
The pre-existing modification to `site/preview.png` is preserved.

This report records the initial findings. See the
[September 14 implementation and verification](autonomy-implementation-2026-09-14.md)
for subsequent fixes and the remaining scope; findings below are historical.

## Verdict

**Klyne is a capable supervised prototype. It is not yet a simple,
reliably unattended computer agent.** The earlier nine phases added useful
infrastructure, but several acceptance criteria were replaced by narrower
implementations. The previous “nothing required” conclusion is not supported
by the current source, even though its environment-dependent tests pass.

The product should let a user describe work, select a project and an access
policy once, and receive a result. Today it still exposes route choices,
manual continuation, ambiguous verification, and recovery mechanics.
Autonomy requires durable execution and verifiable results, not just unlimited
step counts or removing every approval.

This is a source and controlled-fixture audit, not a measured success rate
against arbitrary apps. No personal messages, purchases, account changes,
live model calls, or live MCP installs were performed. Findings below marked
“source” describe confirmed code behavior; their downstream consequences
are stated separately from an executed end-to-end reproduction.

## Findings requiring architectural work

| Priority | Finding and evidence | Consequence and required acceptance check |
|---|---|---|
| P1 | **Failed shell commands lose uncertain effects.** `WorkspaceShellTool` in [tools.rs](../crates/harness-core/src/tools.rs) reports ordinary nonzero exit with `ok:false` and no uncertainty marker. [execution.rs](../crates/harness-core/src/execution.rs) maps that to `NotApplied`; Studio clears pending. Source. | A command can modify a file or remote service and then exit 1. Exit status is not rollback. Add a controlled mutate-then-fail fixture; retain action identity until destination reconciliation establishes what happened. Apply the same contract in Studio and core runtime. |
| P1 | **Completion receipts are not operation-bound.** [completion_guard.rs](../apps/studio/src/completion_guard.rs) accepts any successful action outside a small read-only denylist. Its existing `effect_receipts_confirm_claims` test explicitly lets `browser_click`, `tool_test`, or any shell success confirm “I saved the document.” `list_dir`, `stat_path`, and clipboard reads are also absent from the denylist. | An unrelated action can support a false completion. Bind receipts to action ID, destination, expected effect and fresh verification. Negative tests must reject a save claim backed by an unrelated click, listing, or successful test command. Merely expanding English phrases will not solve this. |
| P1 | **Runtime attestations can be self-declared.** The model-callable `runtime_attest` branch in [chat.rs](../apps/studio/src/chat.rs) checks a binary digest then calls [activation::attest](../apps/studio/src/activation.rs), which stores `test_result:"pass"` without running or looking up the supplied command. Source. | A model can produce a fresh passing attestation without testing. A trusted runner must issue the attestation from its own completed job record and bind the binary/build inputs. A fabricated command string must never qualify a candidate. |
| P1 | **DELETE approval can authorize a template instead of the concrete target.** [local_apps.rs](../apps/studio/src/local_apps.rs) derives `app_invoke` approval from an operation ID such as `DELETE /items/{id}`, before [app_schema::request](../apps/studio/src/app_schema.rs) substitutes parameters. Grants bind connection name rather than immutable origin. Source. | Different item IDs can reuse the same “exact” approval; repointing a connection can change its destination. Build and validate the concrete request first, then authorize origin, resolved path/query and any consequential body. Test two IDs and a changed connection origin. |
| P1 | **New dispatch journal records bypass secret scrubbing.** [chat_store::save_inner](../apps/studio/src/chat_store.rs) inserts `after.clone()` into `action_events` on the dispatch branch. Metadata and the return branch are scrubbed separately. Source. | A known granted secret present in action arguments can remain in the dispatch ledger even if the conversation view is redacted. Scrub every persisted surface before insertion. Use a synthetic secret and inspect all SQLite tables and exports after dispatch and return. This audit did not inspect real credentials. |
| P1 | **Approval policy is both intrusive and incomplete.** [broker.rs](../apps/studio/src/broker.rs) requires exact argv for shell calls, while desktop clicks, browser submissions, non-DELETE API writes and MCP calls do not share equivalent task-level classification. Grants are merge-only and persist across conversation goals. | Normal coding repeatedly interrupts for changed arguments, yet another route can perform a similar effect under a broad toggle. Introduce one user-selected task policy with visible scope, expiry/revocation and consistent route enforcement. Test routine build/edit/test autonomy and a consequential out-of-scope action through every route. |
| P1 | **The normal launcher does not restart crashes.** [klyne-supervisor.rs](../apps/studio/src/bin/klyne-supervisor.rs) exits when its child exits. A separate Python guardian exists, but the README launch command starts only the supervisor. | A dead runtime remains dead without another component/operator. Integrate bounded restart/backoff and crash-loop reporting into the supported launcher. Kill Studio during read, write and idle states; verify continued goals, no duplicate effects, and retained Stop state. |
| P2 | **Long-running work is a blocking call, not a durable job.** [chat.rs](../apps/studio/src/chat.rs) permits `timeout_seconds:0`, but stores no attachable process session, stdin channel or restartable job handle. Process jobs manage descendants, not durable task ownership. | Servers, interactive CLIs and long builds cannot be managed as first-class ongoing work. Provide start/status/output/input/cancel with durable IDs, bounded output and explicit restart reconciliation. |
| P2 | **Tool qualification does not cover executable contents.** [capabilities.rs](../apps/studio/src/capabilities.rs) hashes the saved JSON definition. A referenced Python/PowerShell script can change while program/argv/version remain the same. | “Qualified” means a definition ran once, not that current code was tested. Bind relevant executable/script artifacts and test inputs to qualification; modifying the script must invalidate it. |
| P2 | **Short context and full-history storage do not scale together.** [chat.rs](../apps/studio/src/chat.rs) selects 18 recent messages and six observations, then evicts context above 90 KB. [chat_store.rs](../apps/studio/src/chat_store.rs) loads full history and iterates history entries on every save. | Long tasks lose useful constraints while persistence/polling costs grow. Preserve a structured goal/decisions/artifacts/blockers record, retrieve evidence by relevance, append only new history and paginate UI history. Validate an hours-long task with old constraints and bounded memory/latency. |
| P2 | **Planning is serial and capped at six tasks.** [execution_graph.rs](../apps/studio/src/execution_graph.rs) schedules one ready task; core delegation is not Studio's task engine. Worker evidence binding is recorded but not required. | “Team” presentation does not imply concurrent agents or independently proven steps. Keep serial execution as a sensible default, but add mid-task plan edits and shared-budget delegation only where needed. Test plan changes without losing finished work or the original goal. |
| P2 | **Native browser coverage is narrow.** [browser_tools.rs](../apps/studio/src/browser_tools.rs) exposes CSS click/fill, observation, screenshot, eval/attach and navigation. It checks Stop before dispatch, not throughout blocking CDP operations. | No native semantic locators, tab/frame/dialog management, upload/download contracts or durable browser state. Add those around the same action lifecycle and test cancellation during navigation/read-back and file transfers. |
| P2 | **Desktop latency and coverage remain limited.** [desktop.rs](../apps/studio/src/desktop.rs) launches PowerShell per call and stages C# helpers; the fixture proves only its own controls. Enabling Desktop reserves it for the whole turn, even when a task does not use it. | Unnecessary contention and repeated startup cost hurt autonomous work. Use a supervised persistent helper and acquire input ownership when needed. Qualify DPI/multi-monitor, modal and user-takeover behavior with representative apps. |
| P2 | **MCP is curated and globally serialized.** [mcp.rs](../apps/studio/src/mcp.rs) supports Chrome/Edge adapters and holds a global session mutex through calls. `mcp_setup` can install under Apps access without the shell broker. | Tool discovery is not arbitrary installed-app support. Unify setup authority with task policy; use session-local synchronization and separate connection readiness from operation verification. |
| P2 | **Reviewer access cannot independently verify many external outcomes.** [local_apps.rs](../apps/studio/src/local_apps.rs) limits review to listings/definitions; MCP reviewers cannot call tools and shell reviewers cannot run checks. Delivery keyword checks also reject explanatory requests about sending email. | A reviewer may only reread worker evidence, or falsely block a harmless explanation. Add certified read-only checks and structured requested effects; distinguish “explain how to send” from “send.” |
| P2 | **Budget behavior is per turn, with incomplete metering.** Resuming resets used steps/tokens/cost; defaults are unbounded, and provider cost reporting is not uniformly available. The guard does not reserve the cost of an in-flight model call. | A configured amount is not a hard goal-wide spend ceiling. Show supported meters, retain goal totals, validate limits, and reserve bounded calls. Do not represent unavailable cost data as measured zero spend. |
| P2 | **Release tiers and automation overstate coverage.** The purported headless Tier 0 invokes Chrome/desktop inside Studio. The only repository workflow is [Pages publishing](../.github/workflows/pages.yml). The original baseline script used pipeline status and ignored browser failure in its final exit. | Separate truly headless suites from browser/desktop targets, run gates in CI, and report skipped live suites. A successful publish or local fixture run is not release qualification. Script exit handling and documentation were corrected here; actual tier separation remains open. |

## UX findings and changes made

The default view showed an execution diagram, while CSS hid the conversation
containing Resume and pending-action resolution. Users were told to use controls
they could not see. A visible **Resume work / Review action / Resolve action /
Answer question** button now routes to the relevant action from that view.

The old pending form mixed approval and uncertain execution, defaulted approval
proposals to the invalid “Already completed” choice, required an inspection note
for an action that never ran, and did not display the proposal. It now displays
the exact JSON proposal/action, offers only valid choices, and makes approval
notes optional. Uncertain outcomes still require an inspection note. This is a
functional correction; a plain-language action card should replace raw JSON in
the next UX pass. Approval and continuation remain separate operations.

Desktop lease acquisition previously waited up to 30 seconds while holding the
shared active-conversation mutex. Admission, status and Stop could all stall.
The wait now happens in an admitted worker, checks Stop, and uses the normal
terminal-state cleanup. A regression holds the machine lock and proves that
admission, status and Stop complete without waiting for desktop access.

UI execution settings previously omitted `max_tokens` and `max_cost_usd`, causing
server deserialization to reset configured budgets to zero on Resume. The UI
now preserves those fields. This does not add budget controls or goal-wide
accounting. The workspace heading now preserves `original_request` instead of
replacing the goal with the latest “continue” instruction.

The three pre-existing formatting failures were formatted. The static demo's
shared presentation was rebuilt, and its new continuation control is hidden
when the live-chat controls are absent. The existing preview image was backed
up and restored after tests that write that tracked file.

Remaining UX friction:

- Provider setup, project choice, route toggles, app selection and recovery are
  separate concepts. “Trusted laptop” enables switches but does not explain
  command-by-command approvals or provide a durable task policy.
- Final answers are truncated in the default workspace view; the actual result
  should lead, with activity/evidence behind Details.
- Plain greetings still use planning/worker/review orchestration. A lightweight
  response path would reduce latency and avoid unnecessary machinery.
- New instructions cannot steer active work: users must Stop or wait. Add an
  inbox consumed at action boundaries, preserving goal and completed work.
- Token/spend controls and fallback setup are absent from the normal settings
  surface. The standalone recovery link exposes deployment mechanics.
- Mobile screenshots show readable progress and Stop, but substantial screen
  space is still devoted to the model/worker visualization and route switches.

## Target user experience and implementation order

Default screen: conversation and final result, one composer, project selector,
one access-policy summary, progress sentence and Stop. Details contains plans,
tool traces, provider configuration, budgets and recovery diagnostics.

1. **Make effects and evidence trustworthy.** Fix failed-shell uncertainty,
   concrete request authorization, dispatch scrubbing, executable qualification
   and runner-issued attestation. Add negative tests for each finding above.
2. **Make work durable.** Integrate crash restart, persistent jobs, automatic
   recovery for proven safe operations, durable task budgets and goal memory.
   Stop and unresolved effects must survive every restart.
3. **Make ordinary work autonomous within one policy.** A user authorizes a
   project/task scope once; routine actions continue, and only an actual scope
   change or essential missing fact interrupts. All routes obey the same scope.
4. **Simplify the default UI.** Lead with result/conversation, move route controls
   and orchestration to Details, present actionable plain-language blockers,
   and support steering during work.
5. **Measure the product.** Run a fixed corpus of coding, browser, document,
   cross-app and restart tasks with a real configured provider. Record completed
   outcomes, interventions, duplicate/unauthorized effects, latency and usage.
   Include login-required and unsupported workflows as explicit blocked cases.

A release for unattended use should require zero duplicate or unauthorized
effects in this corpus, correct refusal/reconciliation for ambiguity, usable
Stop/status under contention, and a published completion/intervention rate.
There is no measured general task-success rate today.

## Validation recorded in this audit

- `cargo test --workspace --exclude harness-browser --locked --offline
  --no-fail-fast`: **261 passed, 0 failed, 4 ignored**, exit 0 outside the
  sandbox. Includes the new desktop-contention regression. Log:
  `workspace/audit-ux-autonomy-tests.log`.
- Initial sandbox run failed process/toolchain and Chrome/desktop-dependent
  tests. Outside-sandbox process/delegation rerun: **23 passed, 2 ignored**;
  the complete outside-sandbox run above passed. These are environment failures,
  not proof of product regressions.
- After adding UI assertions, the Chrome production-view test passed. It checks
  visible approval entry, exact proposal text, valid choice sets, preserved
  budgets, desktop/mobile layout, and absence of browser errors.
- Native controlled Chrome suite: **8/8 passed**.
- Rebuilt static demo browser test: **1/1 passed**; the pre-existing preview
  image was restored afterward.
- Windows disposable desktop fixture: **all four PASS checks** (identity,
  field value/truncation, clipboard, stale layout and modal behavior).
- Python recovery suite: **14 run, 1 skipped, no failures**; log:
  `workspace/audit-ux-recovery.log`.
- Workspace Clippy with all targets/features and `-D warnings`: passed.
  Formatting and JavaScript syntax checks passed after changes.
- Baseline-script command fixtures: all-pass exits 0, baseline failure exits 1,
  browser failure exits 1. These validate exit propagation, not real test coverage.
- Live provider quality, live MCP, arbitrary authenticated apps, multi-hour
  soak behavior and the adversarial architectural acceptance cases listed
  above remain unverified. Passing tests does not close those findings.

No commits, pushes, deployment or live runtime replacement were performed.
