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
