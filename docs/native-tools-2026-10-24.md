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

## Review quality and helper model (added 2026-10-24, afternoon)

Settings: `workspace/studio/quality/config.json` (optional; missing keys use defaults):
`{"critique":"auto|on|off","findings":"auto|on|off","checklist":true,"helper":{"enabled":false,"endpoint":"http://127.0.0.1:11434","model":"hf.co/nvidia/NVIDIA-Nemotron-3.5-Bolt-E4B-4GB:Q4_K_M","max_input_chars":12000,"timeout_seconds":90,"internal_jobs":false}}`

- Reviewer findings report (`findings`): one {item, verdict, evidence, severity} per requirement, plus examined / not_examined. A failed medium/high finding blocks completion. `auto` = on for large models, OFF for compact (<=14B Ollama) models.
- Critique pass (`critique`): a skeptical re-check of the reviewer's claims, skipped for chat and single read-only steps. `auto` = OFF for compact models.
- Planner checklist (`checklist`, default on): up to 7 short checks per task.
- Skills: `skill_save` accepts `when_to_use`, `references` (loaded on demand with `skill_read {name, reference}`) and `allowed_tools`; after `skill_read`, that step is limited to the skill's tools.
- Skipped items: reviewer `not_examined` items go to `workspace/studio/skipped.sqlite3` (deduplicated, 14-day expiry) and reach the planner as fenced untrusted data.
- Recovery/helper models are checked against Ollama `/api/tags` at startup; a missing model shows a warning (log + execution settings) instead of a 404.
- `ask_model` {task, input, model?}: read-only helper-model call, no approval; exposed only when the helper is enabled and installed. OFF by default.

Measurements on the MSI (RTX, 8 GB VRAM, qwen3.5:9b, num_ctx 16384, warm):

| Check | Before | With findings + critique on | Final defaults |
|---|---|---|---|
| (a) list C:\temp | 25.2 s | 29.1 s | 29.3 s |
| (b) read a file | 23-28 s | 30.5-57 s (one 104 s run with critique) | 28.9-43.3 s |
| (d) mkdir + write + read back | - | - | 9.6 s to the mkdir approval, 50.3 s after |

The findings report made the reviewer step 11-27 s and the critique added ~38 s, so both are off by default for small models.

Helper (Nemotron 3.5 Bolt E4B, 7.5B Q4_K_M, 3.8 GB in VRAM vs 6.6 GB for qwen): Ollama cannot keep both on the 8 GB GPU and swaps them (Nemotron load 9-16 s, qwen reload 11-14 s). Warm, it was correct but not faster on short jobs because of hidden thinking (intent 1.9 s vs 0.2 s, JSON extract 5.1 s vs 2.0 s). Live delegation "read notes.txt and have the helper model summarize it in one line": 58.3 s (ask_model 13.8 s, 12.9 s of it loading) vs 17.5 s for the same request answered by qwen alone. Not worth it on this GPU; leave the helper off.

## Web reach: transcripts, GitHub links, reader fallback, typed errors (added 2026-10-25)

Adapted from the ideas in [Agent-Reach](https://github.com/Panniantong/Agent-Reach) (MIT licence). Klyne reimplements them natively in Rust (`apps/studio/src/web_reach.rs`); it does not run or depend on Agent-Reach. No cookies, no logged-in scraping, no stealth browser.

- **`fetch_url` routing.** `github.com/<owner>/<repo>` returns the repo facts plus README, `/issues/N` and `/pull/N` return title, state, labels, body and up to 30 comments, via the GitHub REST API. `/blob/...` links read `raw.githubusercontent.com`. Other GitHub pages are fetched normally. A token is used only when the conversation has a `GITHUB_TOKEN` secret grant for `https://api.github.com`; it works without one (60 requests/hour). `youtube.com` / `youtu.be` video links return metadata plus transcript.
- **Reader fallback.** When a direct fetch fails, is near-empty or JavaScript-only, or is a Cloudflare/anti-bot challenge, `fetch_url` retries through Jina Reader (`https://r.jina.ai/<url>`, markdown). Turn it off with `"fetch_fallback":"off"` in `workspace/studio/quality/config.json` (default `"jina"`). 404, 401 and 429 are final and do not fall back. Every fetch records `backend` (`direct`, `jina`, `github_api`, `github_raw`, `yt-dlp`), `route`, `source` and any `fallback_reason` in the evidence. Note: Jina refuses anonymous calls from some datacenter networks (HTTP 401); the original typed error is kept when that happens.
- **Challenge detection.** Challenge pages ("Just a moment...", `cf-mitigated`, captcha/turnstile markers on pages with little visible text) become `PLATFORM_BLOCKED`, and the evidence check never accepts a challenge page as fetched content.
- **`video_transcript {url_or_id, lang?}`.** Uses `yt-dlp.exe` from `tools/yt-dlp/` (install or update with `scripts/install-yt-dlp.ps1`, which checks the release SHA2-256SUMS; the folder is git-ignored). Manual subtitles first, then auto-generated (`<lang>-orig`, then `<lang>`); VTT/SRT become plain text with rolling duplicate lines removed, capped at 11,000 characters, under a Title / Channel / Duration / URL header. yt-dlp runs with `--ignore-config`, a cleared environment, no window, a 90 s timeout and a job object. The tool is not in the compact core set; models reach it with `enable_tools("web_media")`.
- **Typed errors.** Failures carry `error_code` plus a one-line `hint`: `NEED_LOGIN`, `RATE_LIMITED`, `PLATFORM_BLOCKED`, `NOT_FOUND`, `NETWORK`. All but `NETWORK` block the source at once; `NETWORK` blocks it after a second failure. A blocked source gets verify status `blocked` (not unsatisfied), workers and reviewers see it under `blocked_sources` with an instruction not to refetch it, and a repeat request is answered from evidence (`repeat_blocked`, backend `host`) without any network call. The requested URL still passes the normal web permission check; Jina, the GitHub API and yt-dlp are fixed host-owned backends.

### Browser challenges and fetch-first links (added 2026-10-25, afternoon)

- **Challenge detection in the isolated browser.** `browser_open`, `browser_read`, `browser_click` and `browser_fill` observations record `page_url` and `source` (`browser:<url>`). A page whose title/text is an anti-bot challenge (Cloudflare "Just a moment...", "Performing security verification", Imperva, PerimeterX, DDoS-Guard markers, or a blank page titled with its bare domain, as iframe interstitials leave it) becomes `PLATFORM_BLOCKED` with a hint, `ok:false`, verify status `blocked`, and joins `blocked_sources`; the evidence gate never accepts it. One re-read is allowed (a challenge can clear on its own); a later clean read unblocks the source. Klyne does not solve challenges: clicks and fills on a challenge page are refused.
- **No wandering on blocked sources.** A `browser_open` of a URL already blocked by `fetch_url` or the browser, or of a host blocked as `PLATFORM_BLOCKED` / `RATE_LIMITED` / `NEED_LOGIN`, is answered from evidence (`repeat_blocked`, no browser). If the task explicitly needs interaction (clicks, forms, logins, screenshots, or "browser" in the request), fetch-only blocks do not stop the browser, but the browser's own challenge does.
- **Fetch-first.** A `browser_open` of an `https` link in a task that does not need interaction is run as `fetch_url` first (`routed_from: browser_open` in the evidence, a note in the summary). If that text is too thin, the model can call `browser_open` again and the browser opens normally; a blocked fetch blocks the browser too (above). Tool descriptions and browser instructions tell the model to use `fetch_url` for reading links. Reviewers' browser reads are not rerouted; existing-profile and desktop routes are unchanged.
