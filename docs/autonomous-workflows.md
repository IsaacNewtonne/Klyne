# Durable goals and bounded automation

The **Goals** button opens a persistent task board. Add a goal using the current connection and access settings, optionally waiting for unfinished board goals. Open its conversation to see evidence, approvals, steps, and acceptance results. Cancel stops an active worker or removes a waiting goal from execution; it does not undo completed effects. After resolving a blocker in the conversation, use **Recheck result** to inspect the same work without resubmitting it.

The coordinator starts at most two independent goals. Each goal has its own normal planner, worker and reviewer loop, workspace, budgets and checkpoint. Within an individual conversation, the existing dependency graph remains serial. This release does not automatically split every natural-language request into multiple parallel conversations.

Private-file and public-research goals can overlap. Desktop, app API and ordinary Terminal jobs run alone among coordinated jobs. Caller-declared, validated linked Git worktrees can run coding goals concurrently when Desktop and Apps are disabled. Worktrees isolate checkouts, **not OS permissions or external side effects**. Do not assign conflicting deployments, shared databases or shared output paths to concurrent workers. Manually started conversations and schedules retain their existing admission rules; the board is not a global machine sandbox.

Every board goal requires finite step, elapsed-time, token and review limits. Omitted values become 64 steps, 900 seconds, 100,000 tokens and three review passes. Explicit zero is rejected at the API, rather than silently becoming unlimited. The UI explains and applies these background defaults. Provider calls can overshoot a remaining token/time budget while in flight; a zero spend cap still means no separate monetary cap.

## Acceptance and continuation

By default, a board goal requires a caller-owned file contract. Exact-file instructions such as `create file proof.txt with content checked` produce a contract automatically. The existing Report requirements settings provide structural Markdown acceptance criteria. API clients can supply multiple file-content, digest, range or report criteria.

For other goals, explicitly select **Accept model review**. These results remain Reviewed, not independently Verified. A report-format check proves the specified document properties, not that its claims are true, a UI is beautiful, or CI is green. The coordinator rechecks a verified contract against the actual files and confirms the saved goal/contract has not been replaced before completing the board entry. Dependencies only advance after accepted completion. Their bounded summaries are advisory context for dependent workers; private files are not automatically shared between workspaces.

Normal worker/reviewer repair loops, stall detection and uncertainty reconciliation continue inside each goal. The coordinator does not endlessly reset budgets or resubmit blocked goals. Stop, unresolved effects, missing access and failed acceptance require attention. Dispatch is recorded before starting a worker; a crash in that window is marked uncertain and is never blindly replayed. Running jobs retain their conversation IDs across restart. The board is stored in `workspace/studio/goals.sqlite3`.

## Headless jobs

Start Klyne without opening a browser:

```powershell
klyne -NoBrowser
python scripts/klyne-jobs.py submit docs/examples/goals.json --wait-seconds 1800
python scripts/klyne-jobs.py status
python scripts/klyne-jobs.py cancel research-a
```

Set the example's model to an installed model first. The command uses the local Studio API; it does not read provider credentials. Submission IDs are stable and duplicates are rejected. After a lost HTTP response, inspect status before retrying. Wait exits with 0 for accepted completion, 2 for a stopped/blocked result, and 3 when the waiting deadline expires; timing out the client does not silently cancel work.

Jobs execute while the supervisor runs. `-NoBrowser` is background operation, not installation of an operating-system service. Existing scheduled-task settings can repeat a successfully completed conversation with finite limits. Restarting the machine still requires starting Klyne or configuring a user-owned startup job.

## Parallel coding worktrees

```powershell
python scripts/klyne-jobs.py worktree C:\projects\my-app C:\projects\my-app-worker-a
python scripts/klyne-jobs.py worktree C:\projects\my-app C:\projects\my-app-worker-b
```

The source checkout must be clean; no edits are discarded. Each command creates a detached worktree at the current commit. Set `isolation: "worktree"` on each submitted job and its `request.workspace` to the corresponding path, with Terminal enabled. The host checks the Git worktree backlink and prevents two board jobs from occupying the same checkout. Enable autonomous commands deliberately or approve exact commands in the resulting conversation. Each checkout keeps its own build output; shared dependency caches can still help. A shared symlinked target directory is not used. Changes remain available in the worktree for review; creation does not merge, push or activate anything.

## UI audit and repair

```powershell
node scripts/audit-ui.cjs docs/examples/ui-audit.json
python scripts/klyne-jobs.py repair-ui . docs/examples/ui-audit.json --from-chat CONVERSATION_ID
```

The audit captures desktop/mobile screenshots and writes `audit.json`. It checks horizontal overflow plus caller-specified selectors, text, dimensions, exact style tokens and contrast thresholds. Contrast requires opaque computed foreground/background colors; unsupported combinations are inconclusive, not passing. It does not modify the page or certify subjective quality.

The repair command queues a bounded goal to inspect the findings, fix the implementation, rerun checks and write a validation report. The caller-owned report contract verifies report structure only; inspect actual audit results before treating the UI as accepted. Existing browser regression tests now include the goal board and use the same screenshot/check implementation. Screenshots and reports stay under `workspace/studio-qa/` in the examples.

## CI monitoring and instruction review

```powershell
python scripts/klyne-jobs.py watch-ci . RUN_ID --seconds 1800
python scripts/klyne-jobs.py watch-ci . RUN_ID --repair-from-chat CONVERSATION_ID
python scripts/klyne-jobs.py review-instructions . --from-chat CONVERSATION_ID
```

CI monitoring uses an already authenticated `gh` installation. The optional repair flag submits one bounded repair goal, checks that CI belongs to the current commit, and uses a stable run/commit ID to prevent duplicate submissions. It does not automatically push, merge, publish a PR or rerun GitHub CI. A passing local repair is not proof that a new remote CI run passed.

Instruction review produces `instruction-review.md` with source commits, proposed reusable rules, prerequisites, verification steps and conflicts. It does not automatically replace AGENTS.md or promote a model guess to user preference. Use the existing schedule UI for periodic reviews after inspecting the initial result. Existing skill saving and qualified-tool reuse remain available. Optional procedure learning continues to require host-verified file contracts and later successful trials before activation.

## Memory and voice

`memory_save` now accepts `scope: "global" | "workspace"`, `category: "preference" | "environment" | "lesson"`, and optional `ttl_seconds` (1–31,536,000). Automatic recall excludes expired or mismatched workspace notes. Explicit memory reads remain available for inspecting old notes. Defaults preserve existing global advisory memory; nothing is uploaded to a new service, and model-reported notes never acquire host verification from a category label.

Voice status includes running/queued/attention counts from the goal board without an inference call. Conversational context includes bounded board summaries. Voice action handoff fixes remain included. Parallel model requests can still contend on a single local model server; the worker limit is two, not a promise of lower inference latency.

## Validation scope

Deterministic providers exercise real concurrent worker admission, distinct file workspaces, host acceptance and dependency gating. Tests cover duplicate IDs, budget refusal, failed-goal blocking, restart dispatch uncertainty, memory scope/expiry, a real disposable Git worktree, CLI constraints, and the browser goal board. The broader existing regression suites cover recovery, permissions, report acceptance and voice behavior.

Live paid-provider swarms, real authenticated services, an overnight CI repair run, semantic design judgment, and arbitrary signed-in Chrome workflows are not established by these fixtures. No self-rewriting standing instructions, universal autonomous approval, shared-target build guarantee, or cloud transcript database is enabled.

Local Windows validation on October 18, 2026: Studio unit suite 124 passed / 3 ignored; the 79-case chat run initially passed 75, with four browser/desktop failures under the sandbox. All four passed when rerun outside it. The additional real-worktree concurrency test passed, as did the standalone real-browser conversation/settings/mobile test. All three coordinator integration tests passed together after the final host checks. `npm test`, six Python job-client tests, strict workspace Clippy, formatting and diff checks passed. These are local results, not a GitHub CI run.
