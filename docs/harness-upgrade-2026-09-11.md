# Harness upgrade — 2026-09-11

Studio now supports continuing goals, persistent skills/tools/memory, host projects, authenticated app APIs, interactive browsing, and supervised binary replacement. These changes implement the primary capability and reliability work from the [audit](audit-2026-09-11.md). Existing source changes and working files were preserved.

## Start the upgraded runtime

```powershell
cargo build -p klyne-studio --bins --locked --offline
.\target\debug\klyne-supervisor.exe --root workspace/studio --port 4317
```

Open `http://127.0.0.1:4317`. In Settings → Execution and project, choose **Enable trusted laptop access** to enable Web, Terminal, Desktop and App APIs together. This browser preference persists. Individual access switches remain available. Select an absolute project folder to use that project directly; this requires Terminal. Without one, each conversation uses its own files directory.

The supervisor is necessary for `runtime_stage`. Direct `cargo run -p klyne-studio --locked` still works for normal tasks, skills and tools. Stop the earlier server before starting another on the same root/port. A root ownership lock rejects competing servers. Build both binaries before the first supervised launch.

## Continuing work

- New chat turns default to no total step, elapsed-time or review-pass ceiling. Settings accept explicit limits; zero disables that ceiling. Individual operations still have bounds.
- **Resume saved work** retains task state and skips completed tasks. A new ordinary instruction starts a new plan in the same conversation. Completed goals cannot accidentally be resumed as unfinished work.
- On startup, work persisted as Planning/Working/Reviewing/Upgrading resumes when there is no uncertain action. Stopped, Blocked, Completed and uncertain work do not start themselves. Up to four conversations run at once; assigned workers within one goal remain sequential.
- Interrupted external effects retain their pending action. The resolution form records what inspection established: completed, not applied, or abandoned. Resolution itself never repeats the operation. Starting another attempt requires explicit continuation.
- History uses transactional metadata plus individually addressed message/evidence records. Old JSON snapshots remain readable and migrate on save, including snapshots above the former 4 MiB threshold. Conversation listing reads metadata instead of full normalized histories.
- Active context retains recent material and removes older context when needed to fit the provider envelope. Full stored history remains available through the conversation export. This is not unlimited model context or a semantic/vector memory engine.

## Persistent skills and executable tools

Workers with Terminal access can create capabilities themselves:

```json
{"tool":"skill_save","name":"verify-project","description":"Verify this project's changes","instructions":"Read its documented checks, run them, and report actual results."}
```

```json
{"tool":"tool_save","name":"project-check","description":"Run the project checker","program":"python.exe","args":["C:/projects/example/check.py"]}
```

```json
{"tool":"tool_run","name":"project-check","arguments":[]}
```

`capability_list` searches stored descriptions with `query` and pages with `offset`. `skill_read` loads instructions. `memory_save`/`memory_read` store and retrieve reusable findings. `capability_history` lists versions; `capability_restore` activates an earlier version. Capabilities live in `conversations/capabilities.sqlite3`, shared by conversations in that Studio root.

Saving a tool activates its definition immediately; successful execution marks the specific version tested. Updating it creates a new untested version. Tools use argv and append supplied invocation arguments. Use absolute script paths so another conversation can reuse a script. Reviewers can inspect definitions but cannot create, restore or execute them.

This is Klyne's native registry. It does not automatically import Codex's personal skill directories or implement the MCP transport protocol. An external MCP client or app SDK can be wrapped in a registered executable tool. Registry definitions do not grant themselves additional access.

## Laptop and app access

- Terminal now accepts explicitly granted absolute executables. Studio forwards a standard user/toolchain environment profile. `run_shell` also accepts `env` (names to forward) and `timeout_seconds`; zero removes the individual process deadline while preserving Stop.
- Process supervision covers output collection after the parent exits. Windows Job Objects own descendants; cancellation/output overflow/deadline failure returns an uncertain external outcome. The process tool does not provide a PTY or durable interactive stdin session.
- Web includes `browser_open`, `browser_read`, `browser_click`, `browser_fill`, `browser_screenshot`, and `browser_close`. Chrome is isolated by default and sessions survive turns within that running server. `browser_attach` and `browser_eval` require Terminal too. Browser state itself is not restored after a process restart.
- App APIs accept HTTPS origins and numeric private-LAN/loopback HTTP origins. `app_connect` can store `auth.bearer_env` or `auth.headers_env`: environment-variable references, not credential values. The UI exposes the bearer environment reference. Known credential echoes in responses are redacted before entering evidence.
- OpenAPI discovery can use those credentials. HTTP bearer and API-key header declarations are invokable when matching authentication is configured. Changing credentials invalidates saved operation definitions. Complex serialization, arbitrary file uploads, interactive OAuth sign-in and provider-specific login flows still require an adapter.
- Desktop retains screenshot/accessibility control and now uses a cross-process lease. It shares the logged-in Windows desktop. Secure/elevated windows and app-specific controls may need a different integration. App permission grants cannot override Windows or a service's own permissions.

The async HTTP path cancels model and app request waits during connect/send/read. Cancellation cannot undo an already received external request or guarantee that a remote provider stops billing. Codex process cancellation terminates its owned process tree. Browser CDP operations and some Git operations still have their own synchronous completion/deadline behavior.

## Self-improvement and activation

`self_improve` now snapshots a dirty repository into an isolated clean Git repository containing tracked and non-ignored untracked work. Original source and Git state are not reset, committed or merged. Clean inputs still use the original experiment path. The returned `repository` identifies where the candidate branch lives.

Baseline/candidate tests and read-back verification still gate the candidate. The fixed suite remains Cargo-based. Passing test counts do not prove semantic correctness or protect against all modifications to test infrastructure. Test processes have host authority; worktree rollback cannot reverse arbitrary external side effects.

For binary improvements, build and test the candidate, then use:

```json
{"tool":"runtime_stage","binary":"C:/path/to/candidate/klyne-studio.exe","sha256":"<actual SHA-256>"}
```

Staging copies the binary into a version directory, checks its digest, and runs the bounded `--runtime-check` protocol probe. The active goal checkpoints as Upgrading. The supervisor waits for active work to finish, quiesces new admissions, records the previous version, and starts the candidate. It checks health against the new PID. Failed startup returns to the previous binary. The restarted Studio recovers eligible saved goals. `runtime_status` reads the activation result.

`runtime/previous.json`, `current.json`, `last-request.json` and `last-result.json` retain activation evidence. This verifies digest, protocol and startup, not candidate task quality. Activation replaces the Studio binary, not the supervisor itself. Updates must preserve the runtime protocol and stored data compatibility; arbitrary schema migrations need their own compatibility plan. If a tool can be implemented as a script/skill, it can activate directly without binary replacement.

## Core and browser hardening

`delegation_budget::DelegationBudget` supplies persisted atomic child reservations, with `delegation::spawn_child_budgeted` as the integration API. Reservations survive crashes, and unused credit is released only after measured usage is settled. Reopening cannot silently replace capacity. The legacy counter-based `spawn_child` remains available for compatibility; concurrent callers must migrate to the pooled API.

The CDP client now bounds debugger HTTP data, rejects non-loopback debugger addresses, verifies WebSocket Accept/upgrade headers, rejects unexpected response IDs, and queues bounded events. Isolated browser connection failures clean up their process/profile; closing is idempotent. These checks are not an OS sandbox.

## Verification

Regression coverage includes oversized legacy conversation reads and normalized persistence, resolution without replay, a child retaining output pipes after its parent exits, concurrent durable budget reservations, skill/tool reuse across conversations, cancellation during an unresponsive HTTP request, authenticated API discovery/invocation/redaction, dirty-project preservation, restart skipping completed tasks, and real supervisor activation plus failed-startup rollback.

Browser integration uses isolated Chrome and checks the actual Studio UI. Provider orchestration tests use local controlled model endpoints. They establish runtime behavior, not real-model competence on arbitrary multi-hour tasks. No personal app data was edited and no paid model task evaluation was run as part of this upgrade.

Final verification on Windows/MSVC:

- `cargo test --workspace --locked --offline -- --test-threads=1`: **188 passed, zero failed**, two intentionally ignored subprocess fixtures (invoked by their parent regression test).
- `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings`: passed.
- `cargo fmt --all -- --check`, `git diff --check`, and `cargo build -p klyne-studio --bins --locked --offline`: passed; Studio and supervisor binaries are built.
- Desktop (1440px) and mobile (390px) chat screenshots inspected; UI integration and console checks passed.

The initial parallel test run had two Chrome startup timeouts. All eight browser
tests passed in the serialized full run; concurrent Chrome fixture startup remains
a test-environment reliability issue. Real-model task quality and multi-hour
operation have not been established by these tests.
