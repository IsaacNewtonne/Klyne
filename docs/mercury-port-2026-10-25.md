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

## Follow-up fixes (2026-10-25 evening)

Two problems found in live qwen3.5:9b runs after the port.

**Reviewer meta-question.** For a knowledge question ("What colour should I paint a small north-facing room?") the worker answered directly, then the reviewer called `ask_user` with "Is this sufficient, or do you need verification?" three times; the host refused the unsupported question and the chat ended Blocked.
- Root cause: every finished goal went to model review, even when there was no tool evidence to check, and nothing stopped the reviewer handing its own judgement to the user.
- Fix: `turn_end::direct_question` (no URL, path, file, action or live-data words) plus no non-host evidence in the goal window skips review; the worker's answer is shown and a `direct_answer` host record is kept. A reviewer `ask_user` that `turn_end::sufficiency_question` recognises is turned into an accept of the worker's answer (`review_meta_question` record). Real `ask_user` questions for missing information are unchanged. The existing blocker check already stops a repeated unsupported question after three attempts, so the loop guard (which watches dispatched tools) was not changed.

**"Remember that I prefer ..." ended Blocked.** Logged with a local proxy: the planner returned valid JSON; the worker replied in plain text ("Understood. I'll respond in British English ..."). In native-tool mode a short "I'll ..." reply counts as an empty promise and is left invalid, the format retry got the same text, and the chat ended "model response was not valid JSON" before auto-memory could run. Another run wrote an `.acknowledgement.txt` file to "record" the preference.
- Fix: `turn_end::memory_statement` detects remember-this requests. The planner and worker get a host note that memory is saved automatically and only a one-sentence confirmation is needed; a plain-text acknowledgement from the worker on such a request is accepted as the completion; the direct-answer path then skips review and auto-memory records the belief (an existing matching belief gains support instead of a duplicate).

**Second live round.** A first rerun found two more causes:
- One paint run went to the web: the planner planned "consult external web resources", and the worker kept fetching pages for more than 7 minutes. Knowledge questions now carry a host note for the planner and worker: answer directly, no tools unless the user asked for sources.
- One remember run ended Blocked with "Ollama reported an error while generating: XML syntax error ..." (Ollama's own tool-call parser rejected a malformed call). That error is now treated as a format error and retried once with the correction note, instead of being treated as a transport failure with no retry.

**Third fix (9329406).** One remember run still ended Blocked: the worker's `finish` claimed the preference was "stored in memory", the host found no save in the evidence, so the turn went to the reviewer, which asked an unsupported question. For memory statements, a confirmation that claims an unobserved save (or is empty or a refusal) is now replaced by "Noted." on the direct-answer path instead of going to review; auto-memory then records the belief as before.

**Latest live round** (qwen3.5:9b, fresh isolated roots):
- Remember ("Remember that I prefer British English and short replies.") x5 on 9329406: 5/5 Completed in 5-10 s; one belief, support 5, no duplicates.
- Paint question x3 on ab926d6: 3/3 Completed in 10-15 s, no review, no browsing.
- GitHub issue summary on ab926d6: Completed in 35 s.
- Click 'Learn more' on example.com: unreliable; see [KNOWN-ISSUES.md](KNOWN-ISSUES.md).
