# Klyne Studio

A local visual workspace for the Rust agent harness. Studio ships as one Rust
binary with its HTML, CSS, and JavaScript embedded. No Node build, external
fonts, frontend dependencies, or hosted service are required.

## Launch

```powershell
cargo run -p klyne-studio --locked
```

Open http://127.0.0.1:4317. Options: `--port 4317 --root workspace/studio`.
The server binds to loopback only. Use the printed IP address, not localhost.

## Chat workspace

The home page opens directly to the conversation. Enter an instruction and send
it; worker messages and tasks are under Details. Access, project, command policy
and budgets live in Settings; the composer shows an access summary. Model connection settings live in a dialog;
Codex is the default, with Ollama and OpenCode available. The built-in file
adapter is not an AI and is not offered for chat.

Studio calls an AI planner, one to six sequential worker roles, and a separate
read-only reviewer. Workers share the selected project and grants. These are
separate calls to the selected provider, not the core delegation scheduler.
Written files receive exact read-back checks; broader completion is model-reviewed.
Each role receives a host-generated policy snapshot. Tasks awaiting approval
pause their dependents while other ready tasks continue; the next approval card
appears when that independent work finishes.

New turns default to no total step, elapsed-time or review-round ceiling. Settings
can set explicit limits; zero disables that ceiling. Individual operations retain
output and timeout bounds. Stop cancels supervised processes and model/API HTTP
calls. Some synchronous browser/Git operations finish within their own timeout.
Token and estimated-cost budgets are also available; zero disables them. These
are driver limits, not guarantees of provider billing accuracy.
Resume preserves the goal's usage counters; increase an exhausted budget in
Settings to continue. A new goal starts fresh counters.

History is persisted transactionally in metadata and individual message/evidence
rows under `<root>/conversations/<id>/chat.sqlite3`. Recent context is bounded;
the full history remains exportable. Startup resumes eligible unfinished work.
**Resume saved work** skips completed tasks. Uncertain actions require inspection
and recorded resolution before continuation; recovery never automatically replays
them. An exclusive root lock prevents competing servers.

Settings can select an absolute host project (requires Terminal), configure limits,
or enable all four access routes and autonomous commands with **Enable trusted laptop access**.
New conversations default to asking for exact command approval. Selecting
**Run autonomously** lets Terminal execute commands without individual prompts;
secret transfers and destructive API calls retain their separate approvals.
**Approve and continue** records the displayed proposal and resumes automatically.
Cards expire after 15 minutes and bind the current policy and relevant command
inputs. Expired or changed requests require a fresh proposal. See
[shared policy and approval lifecycle](shared-policy-and-approvals.md) for the
contracts, compatibility details and enforcement limits.
Failed commands that may have changed external state stop for inspection.
Terminal runs
commands with host authority and a user/toolchain environment profile. Web offers
HTTPS fetch and interactive isolated Chrome; attaching to existing CDP browsers
and evaluating JavaScript additionally require Terminal. Desktop operates visible
Windows apps with screenshots and accessibility observations, a cross-process
exclusive lease, and Pause/top-left-corner/Stop controls. App APIs support HTTPS
and numeric private-LAN/loopback HTTP, OpenAPI discovery, and environment-backed
bearer/header credentials. Installed apps remain available through the Apps picker.

Terminal-enabled workers can save, edit, test and restore versioned skills,
executable tools and memory shared across conversations. They can test code
changes in isolated experiments, including snapshots of dirty projects. Start
through the supervisor for staged runtime activation, startup rollback and up to
three rapid crash restarts before a crash loop stops:

```powershell
cargo build -p klyne-studio --bins --locked
.\target\debug\klyne-supervisor.exe --root workspace/studio --port 4317
```

The [upgrade guide](harness-upgrade-2026-09-11.md) documents the tool contracts,
recovery behavior, verification and remaining limits. App sign-in and compatibility
are app-specific; the native capability registry is not an MCP transport.

The [September 14 implementation notes](autonomy-implementation-2026-09-14.md)
describe the current approval, qualification, executed-test attestation and
completion-receipt contracts, including migration from earlier records.

The chat browser test verifies actual file creation through a scripted local
model endpoint, conversation rendering, escaping, draft retention, settings,
responsive layout and console errors. Integration tests verify a full repair
cycle, follow-up context, disabled capabilities, reviewer write denial, and
Stop during a model call. Unit tests verify interrupted persistence and refusal
to replay pending effects. Scripted endpoints verify orchestration mechanics;
they do not measure real-model quality.

## Prompt maker preset

“Find the right words” selects Prompt maker in the conversation mode selector.
The preset instructs the planner, workers and reviewer to produce concise,
copy-ready prompts with clear objectives, context, constraints and output
formats. It preserves user intent, labels assumptions, avoids invented facts,
and asks targeted questions when essential information is missing. Supplied
prompts are editing material, not tasks to execute. The mode persists with the
conversation and can be switched back to General assistant.

Ollama requests in this mode include `temperature: 0.2` and `top_p: 0.5` through
its [compatible endpoint](https://docs.ollama.com/api/openai-compatibility).
The current Codex CLI and OpenCode connectors do not expose per-request sampling
controls; they receive the behavioral instructions, and the UI states this
limitation. General assistant retains provider defaults. Sampling settings
cannot guarantee factual correctness.

Validation: all 15 Studio tests pass, including persisted mode, instruction and
sampling payload checks, general-mode isolation and browser preset selection.

## Legacy file workspace (`/files`)

A compact ivory and sage workspace with subtle motion:

- **Make something** — file path, file contents, tool-call budget, Launch button.
  Choose the built-in agent, Ollama, OpenCode, or Codex in **Run with**.
  See [local provider setup](studio-providers.md) for connection and model options.
  Each run gets its own `files/` workspace and SQLite database under the
  Studio root.
- **Runs** — filter box and a list of the latest 100 runs with their status.
- **Run details** — status, tool-call count, Stop/Resume/Export buttons,
  an error line when something fails, an Evidence list of tool observations,
  and a filterable event ledger (latest 200 events, details capped at
  8,192 characters).
- **Refresh** — re-reads local state; the page also polls every 1.5 seconds
  while visible. Export downloads a JSON snapshot of the selected run.

## Boundaries

The form creates exact-content file objectives. The default is the deterministic
file agent; optional Ollama, OpenCode, and Codex adapters propose decisions
through the same runtime. Runtime shell/network tools remain denied. Selected
providers communicate through local HTTP or the Codex CLI and may use cloud
inference. See [provider boundaries](studio-providers.md). The local API requires the exact
Host and same Origin when provided, rejects cross-site requests, and requires
a custom header and JSON for mutation. It is not a multi-user authenticated
service. Use a trusted local root; runtime filesystem checks are not an OS
sandbox. Requests are size bounded, with socket timeouts; HTTP clients are
handled serially. Export contains the displayed snapshot, not the full event
database.

## Verification

`cargo test -p klyne-studio --locked` exercises local API rejection, actual
file execution, read-back, event/run filtering, a real Chrome export download,
server-restart persistence, refusal to replay terminal runs, and stop/resume
wiring against a real persisted checkpoint. Browser tests require installed
Chrome and permission to launch an isolated headless instance.

Screenshots from tests are saved under `workspace/studio-qa/` (git-ignored).

## Live connection check — 2026-09-10

The installed Codex CLI login check passed. A real, tool-free welcome request
completed through planner, worker and reviewer in three model calls on the
running Studio server. This confirms the live chat connection and orchestration;
complex goal quality and non-Codex live chat providers were not evaluated here.

## Local API and improvement continuation

Local APIs enables reusable loopback HTTP app connections. The Apps panel manages
saved connections and learns documented operations from OpenAPI descriptions.
Workers can search and invoke those saved operations. Terminal also enables
isolated improvements containing implementation and test files; Stop interrupts
suite execution. See [local app tools and limits](local-apps.md).
