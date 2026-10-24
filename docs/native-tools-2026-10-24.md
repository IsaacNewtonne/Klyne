# Native tool calling, small-model mode and destructive approvals (2026-10-24)

This change makes Klyne usable with small local Ollama models such as `qwen3.5:9b`.
Before it, the worker had to return a hand-written JSON decision inside a very large prompt.
Small models often answered in prose such as "I can't access your files" or "On it!", and nothing ran.

## What changed

- **Native tool calls (Ollama).**
  - The worker and the independent reviewer now receive Ollama's native `tools` list and reply with `tool_calls`.
  - Each call is mapped onto Klyne's existing decision/action structures, so access checks, approvals, receipts and the reviewer still apply unchanged.
  - A strict `<tool_call>{...}</tool_call>` text fallback is accepted.
  - Prose is never executed.
  - `<think>` blocks are stripped.
  - Refusals ("I can't access…") and empty promises ("On it!") are rejected and retried, not shown as answers.
- **Requests to Ollama.**
  - Streaming.
  - `keep_alive: "30m"`.
  - `options.num_ctx: 16384` (32768 if a prompt is larger).
  - The UI shows loading-model, thinking and generating phases with elapsed seconds.
- **Compact tool mode.**
  - Settings → Provider → *Tool mode*: Automatic (default), Compact or Full.
  - Automatic uses compact mode for Ollama models of 14B parameters or fewer.
  - Compact mode sends short role prompts, a clipped context and the core tools.
  - The model can call `enable_tools` to add a tool group (for example `browser_developer`).
- **Routing.**
  - Voice and greeting shortcuts now apply only to genuinely conversational input.
  - Requests with action verbs, paths or tool nouns go to the planner.
  - A voice-route failure falls through to the planner.
  - A named file path selects its parent folder as the workspace.

## Confirm before destructive actions

The host, not the model, pauses these actions for approval:

- `delete_path`
- `move_file` or `copy_file` onto an existing destination
- destructive `run_shell` commands when the command policy is Autonomous:
  - deletes and formats
  - registry edits
  - shutdown and restart
  - process kills
  - git force push, `reset --hard` and `clean -f`
  - `setx`
  - package install and removal
- desktop clicks or invokes on send, buy, pay, post, submit, delete and similar controls, and Enter in messaging or shopping windows
- `runtime_stage`, which activates a self-update

The approval card offers three choices:

- **Approve once.** Binds to the exact action (sha256 of the action) and is consumed on use.
- **Approve for this conversation.** Covers the same category, such as every delete, in this conversation. It expires with the normal 15-minute approval lease.
- **Decline.** Nothing runs.

Not gated yet: `desktop_type` (typing text).

## Real Chrome (opt-in, off by default)

- Settings → *Use my real Chrome*.
- When it is on and web and terminal access are enabled, the worker is told it may attach to `http://127.0.0.1:9222`.
- Start Chrome for this with `scripts/start-chrome-debug.ps1`. It refuses if Chrome is already running.
- Caveat: Chrome 136 and later refuse remote debugging on the default profile. Use a separate `--user-data-dir`, which means separate logins.
- Klyne never reads cookie or password files.

## Launcher warm-up

`scripts/start-klyne.ps1` now preloads the model in a hidden process before the build check:

- `-WarmModel <name>` sets the model. The default is `$env:KLYNE_WARM_MODEL`, then `qwen3.5:9b`.
- `-NoWarmup` skips the warm-up.
- The warm-up uses the same `keep_alive` and `num_ctx` that Klyne sends.

Recommended user environment variables for Ollama:

- `OLLAMA_KEEP_ALIVE=30m`
- `OLLAMA_CONTEXT_LENGTH=16384`

## Measured on the development PC (8 GB GPU, qwen3.5:9b, Ollama 0.40.4)

- Model warm-up: about 10 s.
- At 16K context the model takes 6.59 GB. 5.52 GB of that is in VRAM and about 1.06 GB is in system RAM, so it is not a full GPU fit.
- Wall-clock times for live tasks:

| Task | Result | Time |
|---|---|---|
| List `C:\temp` | Completed | 25 s |
| Read a named Desktop file | Completed | 23 to 28 s |
| Follow-up "delete that file" | Paused for approval; the file was not deleted | 11 s |
