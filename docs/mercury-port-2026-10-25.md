# Ideas adapted from Mercury Agent (2026-10-25)

Five behaviours below are adapted from
[Mercury Agent](https://github.com/cosmicstack-labs/mercury-agent)
(MIT License, © 2026 Cosmic Stack). Klyne reimplements them natively in
Rust; it does not depend on or vendor Mercury's code. Every Klyne safety
rule stays in force: approvals, receipts, the evidence and claim checks,
and the existing step, token, spend, time and review limits.

## 1. Loop, repetition and stall detection

Module: `apps/studio/src/loop_guard.rs`, wired into the worker loop in
`chat.rs`.

| Condition | Soft trip (warn once) | Hard trip |
|---|---|---|
| Same tool, identical arguments, in a row | 4 (read/fetch tools: 6) | 2 more after the warning |
| Same tool failing (anywhere in the last 40 calls) | 6 (read/fetch tools: 8) | 2 more after the warning |
| Worker steps in a row with no tool action | 4 | 6 |
| Near-identical replies (word overlap > 75%) | 3 | 5 |
| Tool calls per goal | – | 75 (cap) |
| Failed tool calls per goal | – | 20 (cap) |

* A **soft trip** adds a `loop_warning` observation and a one-shot
  `loop_warning` field for the next worker call.
* The first **hard trip** hands the step to the reviewer (the existing
  stalled-task review), then clears the recent pattern. A second hard trip,
  or a cap, ends the turn as **Needs input** with the reason
  (`loop_stop` evidence). Nothing is marked Done by the guard.
* Read and fetch tools (`read_file`, `list_dir`, `fetch_url`,
  `browser_read`, `desktop_observe`, …) get the higher thresholds.
* The guard sits beside the existing caps; it does not replace the
  user's step limit, the file-operation journal (`progress.rs`) or the
  review-round limit.

**Stall watchdog.** Every saved step (model stream, tool result, step
change) is a heartbeat. After `soft_seconds` (default 180 s) with no
heartbeat the UI shows "Still working · no new output for N min". After
`hard_seconds` (default 480 s) Klyne stops the current call and pauses as
Needs input with "I paused because nothing progressed…". Configure in
`quality/config.json`:

```json
{"stall": {"soft_seconds": 180, "hard_seconds": 480}}
```

## 2. Turn-end verdict and narration guard

Source idea: Mercury Agent's completion verdict (MIT), reimplemented in
`apps/studio/src/turn_end.rs`.

Every run now records `execution.turn_end`:

| Verdict | When | Status shown |
|---|---|---|
| `chose_to_stop` | the model finished and the review passed | Completed |
| `steps_exhausted` | the goal's step budget ran out | **Needs input** (a pause, never Done) |
| `interrupted` | the model server/connection failed or timed out | Blocked |
| `truncated` | Ollama ended the reply with `done_reason: "length"` | Blocked |
| `aborted` | the user pressed Stop (or the stall watchdog paused it) | Stopped / Needs input |
| `blocked` | any other error | Blocked |

Step budget: the message says the task is NOT done. The step count is not
reset on resume (same as before); after raising the limit and pressing
Resume work, the first planner/worker call gets a one-shot
`resume_nudge`: "you are NOT done, resume where you left off". There is
no automatic resume, so every continuation still goes through the user
and the approval queue.

Narration guard: when the user's request asks for a change (save, export,
download, install; write/create/edit/... plus a file target; run/build
only with Terminal on) and the worker tries to finish with no write,
command or receipt in this goal, the finish is refused (`narration_check`
evidence), and the next worker step is forced to act: in compact mode
only acting tools plus `ask_user` and `report_blocker` are offered
(`finish` and read tools are withheld for that step). After three refused
finishes the task pauses as Needs input. Goals with an explicit task
contract keep the contract check instead.

## 3. Token-budgeted context window

Source idea: Mercury Agent's context window (MIT), reimplemented in
`apps/studio/src/context_window.rs`.

- **History** is filled newest-first inside a token budget (chars/4):
  3000 tokens in compact mode (small local models), 6000 otherwise, sized
  so system prompt + tools + observations stay inside Ollama `num_ctx`
  16384. The latest user message and everything after it are always kept
  whole; older messages are clipped to 1500 characters and dropped once
  the budget is spent (this replaces the fixed "last 18 / last 10").
- **Observations** shown raw are the newest 6 from the *current goal*
  only. If the goal has fewer than 3, the previous goal's newest
  successful results are carried raw so follow-ups ("personalize it")
  still see them.
- **Older tool activity** (earlier in the goal and earlier goals) is sent
  as `earlier_tool_activity`, up to 12 one-line traces:
  `read_file notes.txt → ok (3 lines)` / `fetch https://… → ✗ HTTP 404`.
- **Host notes** (loop warnings, the step-budget resume nudge and, from
  piece 5, remembered beliefs) go into `system_notes`, which the Ollama
  client sends as a second system message ("HOST NOTES …") instead of
  inside the history JSON.
- Evidence, receipts, the completion gate and claim checks are unchanged:
  they still read the full evidence list, not the prompt window.

## 4. Automatic tool groups

Source idea: Mercury Agent's tool exposure (MIT), reimplemented in
`apps/studio/src/native_tools.rs` (`AUTO_GROUPS`, `auto_groups`, `hint`).

In compact mode (small local models) the always-on core is now: workspace
files, fetch-first web (`fetch_url`, `browser_open/read/click/fill/close`
— click and fill stay core for the click gate), `run_shell`/`ask_model`
when Terminal is on, `runtime_status` and `capability_list`. Other groups
are sent only when the original request, the current assignment or the
last 4 user turns contain one of their keywords:

| Group | Tools / host groups | Example keywords |
|---|---|---|
| memory | memory_read, memory_save, memory_forget | remember, recall, forget, prefer, from now on, my name |
| skills | skill_*, tool_save/test/run/forget, capability_history/restore | skill, tool, script, reusable, automate |
| apps | app_list + the apps group | app, connector, mcp, slack, github, api |
| desktop | desktop_observe/apps/launch/focus/click/type/key | window, screen, notepad, launch, click, press |
| web_media | video_transcript | youtube, video, transcript |
| browser | browser_screenshot + browser_developer | screenshot, console, inspect, scroll |
| documents | documents + resources groups | pdf, docx, xlsx, csv, spreadsheet, report |
| voice | voice group (when offered) | voice, speak, read aloud |

`enable_tools` still exposes any hidden group explicitly (its description
lists what was auto-shown), access toggles still decide what exists at
all, and full (non-compact) mode is unchanged.

## 5. Auto-memory (beliefs)

Source idea: Mercury Agent's memory extraction (MIT), reimplemented in
`apps/studio/src/beliefs.rs`.

- **When.** After a task ends **Completed** (not on stop, pause or
  failure), once the reply is saved, so it never delays the answer. It is
  skipped for trivial turns (hi, thanks, ok, continue…), when the user's
  words this turn carry no first-person or preference signal, when a
  `memory_save` already ran this turn, and for non-Ollama providers.
- **Model.** One non-streaming call to the configured worker model
  (`models.worker`, else the conversation's model) with `think:false`,
  JSON mode, `num_predict` 400 and the same `num_ctx`/keep-alive as the
  worker. The worker model is already loaded, so there is no extra swap
  on an 8 GB GPU.
- **Input.** Only the user's own messages from this goal (each clipped to
  1500 characters; messages that look sensitive are dropped first) plus
  the known active beliefs. It never sees tool outputs, web pages or the
  assistant's replies.
- **Output.** A JSON object `{"ops":[…]}` (Ollama JSON mode always returns an object; a bare array or a single operation object is also accepted) with at most 5 operations:
  `create`, `owner_stated`, `support`, `contradict`, `correction`. Kinds:
  fact, preference, trait, situation, relationship, goal, project,
  episode. Statements are at most 200 characters. Invalid operations are
  dropped.
- **Filter.** Statements that look like passwords, keys, tokens, seed or
  recovery phrases, OTP/2FA codes, logins, card/bank/ID numbers, emails,
  phone numbers, addresses, birth dates, health, religion or orientation
  are dropped. The filter checks for `@`, 7+ digit runs and long random
  strings, and reuses the existing memory secret check.
- **Lifecycle.** A new belief starts at confidence 0.55 (0.8 if the user
  stated it). A near-duplicate (>75 % word overlap) adds support instead.
  Contradiction lowers confidence and retires the belief once the
  contradictions reach the support count. A correction retires the old
  belief and creates the corrected one. Confidence halves every 90 days
  without support, and a belief below 0.2 is retired. Beliefs are
  **retired, never deleted**. They are stored in
  `workspace/studio/memory/beliefs.json`.
- **Use.** Up to 5 relevant active beliefs (preferences first, then those
  sharing words with the request) go into the planner and worker host
  notes. They are labelled as possibly outdated, and the user's current
  words win.
- **UI and API.** Under **Settings → What Klyne remembers about you**,
  each belief shows its kind, confidence and support, with a **Retire**
  button. `GET /api/beliefs` lists them; `POST /api/beliefs
  {"action":"retire","id":"b1"}` retires one.
- **Config.** `"auto_memory": true` (default) in `quality/config.json`;
  set `false` to turn it off. The `memory_read`/`memory_save`/
  `memory_forget` tools and their sqlite store are unchanged.
