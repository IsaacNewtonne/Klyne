# UX and autonomy implementation — September 14, 2026

This local change follows the [source audit](audit-2026-09-13-ux-autonomy.md)
against `a02895a`. It closes several concrete gaps behind the earlier release
claim. It does not establish reliable unattended operation across arbitrary apps.

The subsequent [shared policy and approval lifecycle](shared-policy-and-approvals.md)
adds policy inheritance, expiring cards, independent-task continuation and budgets
that persist across approval resumptions. Its test results are separate from the
269-test implementation baseline recorded below.

## User experience

Studio opens the conversation by default. Worker messages, task breakdowns and
evidence are under Details. Settings contains access routes, project selection,
command policy and step, time, review, token and estimated-cost limits. The
composer exposes an access summary. Saved conversations restore their settings;
partial API updates preserve unspecified limits.

New conversations ask for exact command approval. Users can explicitly select
autonomous command execution for that conversation. Trusted laptop access enables
this policy alongside the access routes; an old saved trusted-access preference
does not silently enable the new policy. Terminal still runs with the user's
host authority. This is not an OS sandbox.

Approval cards explain the operation, offer the exact request in expandable
details, and resume with one **Approve and continue** click. Approval notes are
optional. Uncertain outcomes require an inspection result instead of approval;
recording a result does not automatically repeat the action. Activity view also
exposes continuation when a run needs attention.

## Execution and evidence

- Nonzero shell exits retain a pending uncertain action across restart. A command
  can change files before failing, so it is not automatically replayed. The
  benchmark's disposable Cargo repair fixtures have a host-owned exception for
  their exact test argv and exit 101; this does not grant execution or assert
  that failed diagnostics had no effects.
- DELETE approvals bind the resolved origin, concrete path and query, method,
  connection and body digest after OpenAPI parameter substitution. Changing
  an item ID, query, body or origin invalidates approval.
- Known secrets are scrubbed before new metadata, history, evidence and
  dispatch/return journal rows are persisted. Existing historical rows are not
  retroactively migrated or scrubbed.
- Saved tool qualification binds full argv, the resolved executable, file
  arguments and explicitly declared `artifacts`. Inputs are checked before and
  after execution. Changed inputs require qualification again. This does not
  automatically discover every imported or dynamically loaded dependency.
- Runtime attestation executes a supervised test command and requires success,
  an unchanged candidate digest and a host-recorded output digest. Starting a
  rerun invalidates the old receipt, including when the rerun fails. A zero exit
  proves a successful process run, not adequate tests or correct software.
- File writes and patches produce independently checked digest receipts.
  Completion checks match effects and named targets and recheck current file
  contents; unrelated successful tools cannot substantiate a save or send claim.
  Structured reviewer claims must match host receipts. General prose detection
  remains heuristic, and arbitrary browser, desktop and shell effects do not
  have universal verification adapters.
- The supervisor restarts a crashed child up to three times during a rapid
  crash loop, records restart status and stops after the fourth crash. A healthy
  minute resets the count. Existing checkpoint recovery handles eligible saved
  work; uncertain actions remain paused. This is not a durable detached-job or
  PTY manager.
- Desktop lease acquisition no longer holds the shared chat-admission mutex;
  waiting conversations remain stoppable and status remains responsive.

## Compatibility

Legacy DELETE grants without origin/body binding and old tool qualifications
fail closed and need renewal. Legacy self-declared runtime attestations no longer
qualify a candidate. The chat `runtime_attest` tool now accepts
`test_argv: [program, argument, ...]` instead of a descriptive `test_command`.
The supervisor's `--attest-command` accepts a JSON argv array and executes it;
`--binary` must match `--attest-digest`. Receipt files are local same-user state,
not tamper-proof records against a process with host filesystem authority.

## Verification

The mixed workspace suite passed **269 tests, zero failures, 10 ignored** with
`cargo test --workspace --exclude harness-browser --locked --offline --no-fail-fast`.
The ignored entries include subprocess fixtures and opt-in live tests; they are
not reported as passes. Studio's browser tests run real Chrome against scripted
local model endpoints. The approval test additionally clicks the real UI and
observes completion without a separate Resume action.

Formatting and workspace Clippy with all targets, all features and warnings
denied pass. Logs are in `workspace/autonomy-implementation-final-tests.log`,
`workspace/autonomy-approval-browser.log` and the `workspace/autonomy-*-gate.log`
files. Additional gates: controlled browser **8/8**, approval-and-continuation
browser test **1/1**, recovery Python suite **14 run, one skipped, no failures**.
The desktop fixture initially failed because Windows refused foreground focus;
one isolated retry passed all four checks. That environment-sensitive failure is
retained in `workspace/autonomy-desktop-gate.log`, with the passing retry in
`workspace/autonomy-desktop-gate-retry.log`. It must not be represented as a
uniformly green first run.

No live provider quality evaluation, authenticated-site task, personal message,
purchase, deployment or unattended installation was performed. The pre-existing
`site/preview.png` modification is preserved. Changes remain local.

## Remaining work

Persistent desktop supervision, durable background jobs/PTYs, browser semantic
locators and transfer workflows, more host-verifiable effect adapters, broader
consequential-action classification, relevance-ranked compaction and measured
real-model task success remain separate architectural work. The controls above
improve autonomy within the implemented routes; they do not remove these limits.
