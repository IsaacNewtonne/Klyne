# Recovery, review models, and scheduled tasks

Studio now provides three additional controls through its existing execution
runtime. No new model weights or external agent harness are required.

## Separate review model

Settings → Review model selects Codex, Ollama, or OpenCode independently of the
worker. “Same as worker” retains the previous behavior. The connection is saved
with each conversation and survives restart and Resume. Explicit changes in
Settings apply on the next submission; new conversations use the browser's
saved review default. Planner and worker continue using the main connection.

Both final review and stall review use this selection. Review remains read-only
and consumes the same goal's steps, elapsed time, tokens and reported cost.
A dedicated review connection never silently falls back to the worker/global
fallback connection. A separate model is another opinion, not independent proof:
host acceptance checks and existing destination-evidence rules still apply.
Tasks independently satisfied by the existing host-contract fast path do not
need a model review call.

## Recurring file operations

The original fresh-hash check for identical file writes remains in place.
An additional durable, per-task history records hashes of completed file-tool
requests and their observed results. It recognizes a proposed action whose most
recent result has recurred three times within eight tracked operations, including
repeated reads and short cycles of alternating file contents.

Detection asks a read-only reviewer to inspect the current task. It neither
declares success nor replaces remaining tasks or dependencies. Two recovery
reviews are allowed without a newly observed action/result pair; continued
repetition blocks with unfinished work preserved. Resume does not clear the
counter. A new task or goal starts a new history. Only hashes are retained in
this journal; the normal evidence store retains the underlying observations.

This is a bounded heuristic, not a universal progress measure. Different path
spellings, long cycles, changing timestamps, and results outside the retained
window can evade it. Shell, desktop, browser, remote-job polling and API actions
are excluded. Uncertain effects keep their existing reconciliation behavior.

## Scheduled tasks

Select a completed general-assistant conversation, open Settings → Scheduled
tasks, and choose an interval. The first run starts after that interval. Each
run creates a separate conversation with the source task's saved instruction,
models, access, project and acceptance contract. Pending actions and approval
grants are never copied. Use a self-contained instruction for repeatable work;
the source conversation's discussion is not copied into each new run.

With Terminal access, runs reuse the saved project folder. Without a selected
host project, runs use fresh conversation workspaces. Unlimited step/time/token/
review settings become 64 steps, 600 seconds, 100,000 tokens and three review
passes; explicit nonzero settings and reported cost limits are preserved.

The scheduler runs once per second while Studio is running. Closing the normal
Klyne window stops it; the existing `klyne -NoBrowser` launcher keeps the server
running without an app window. This does not install an OS startup service,
wake a sleeping computer, or provide filesystem/webhook triggers.

Schedules are durable SQLite records with these transitions:

`Waiting → Dispatching → Running → Waiting / Needs attention`

- A dispatch claim commits before a conversation starts. A crash in that gap
  pauses the occurrence for inspection rather than risking duplicate effects.
- Active runs never overlap within a schedule. The next interval starts after
  completion; missed intervals coalesce into one run rather than a backlog.
- Blocked, stopped, missing, approval-waiting and failed runs pause future runs.
- Pause prevents future dispatch and does not stop an active conversation.
  Use that conversation's Stop button to stop it.
- Enable future runs schedules a fresh occurrence after the interval. It does
  not resume old work and refuses while an old run is active or unresolved.
- Runtime upgrades quiesce scheduled admission along with interactive work.
- Removing a schedule retains its conversations and artifacts.

Routes: `GET /api/schedules` lists jobs; `POST /api/schedules` accepts `create`
with `chat_id`, `interval_seconds` (60–31,536,000) and optional `name`, or
`pause`, `enable`, `delete` with `id`. They use the existing local HTTP request
and origin checks. No schedule-creation tool is advertised to the planner.
Terminal access retains its existing same-user authority; these routes are not
an OS security boundary. A scheduler persistence error stops its dispatch thread
and is reported on Studio's stderr; restart after resolving the storage problem.

These additions do not automatically merge improvement experiments or enable
unattended authority for existing conversations.

## Validation scope

Deterministic fixtures cover repeated reads, alternating-write recognition,
recovery exhaustion across Resume, separate reviewer routing and read-only
context, schedule persistence across a process restart, duplicate prevention
during a delayed run, pausing on missing input, and an interrupted dispatch claim.
The browser settings test covers review-model persistence, conversation-specific
overrides, clearing the old conversation URL on New chat, and schedule creation.

```powershell
cargo test --locked -p klyne-studio --bin klyne-studio --test chat --test runtime -- --test-threads=1
cargo clippy --locked -p klyne-studio --all-targets -- -D warnings
cargo fmt --all -- --check
npm test
```

The model fixtures exercise control flow; they do not measure a real model's
judgment or establish unattended task-success rates. No specialist model was
downloaded, no live user task was scheduled, and no running user app was restarted
as part of this implementation.
