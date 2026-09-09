# Supervised process execution

Status: **IMPLEMENTED** as an authorized-but-not-isolated supervisor. OS isolation remains **PLANNED**.

## What changed

- Structured tool descriptors: every tool exposes `ToolDescriptor { tool, description, actions }`
  with per-action effects (`read-only` / `mutating` / `external`). `ToolRegistry::descriptors()`
  is the discovery surface; the CLI prints it via `--tools`.
- `RunShell` executes executable + argv only. No shell string is ever concatenated.
  Working directory is fixed to the workspace root; there is no caller-controlled cwd.
- Explicit grants: the shell capability defaults to disabled. `allow_shell_program()`
  grants one executable with unrestricted argv; `allow_shell_with_arg_prefix()` restricts
  argv to registered prefixes. Program names containing `/`, `\`, or `:` are denied,
  as are unknown executables.
- Environment is cleared (`env_clear`) and only explicit `allow_env()` grants are
  forwarded, plus non-secret OS-minimum entries required for process creation and
  executable lookup (`PATH` everywhere; `SYSTEMROOT`/`WINDIR`/`COMSPEC`/`PATHEXT` on
  Windows). Secrets must never rely on ambient environment.
- Bounds: 30 s default timeout and 64 KiB per-stream stdout/stderr caps
  (`ProcessLimits`, configurable per registry via `milestone_with_shell_limits`).
  Timeout kills and reaps the child; excess output fails the call with bounded evidence.
  Partial output on timeout is truncated to the per-stream cap.
- Windows-first tree cleanup: on timeout/supervision failure the runtime runs
  `taskkill /PID <pid> /T /F` before `kill()`/`wait()`. Unix currently terminates
  the direct child only; full process-group cleanup is planned.

## Verified

New `crates/harness-core/tests/process.rs` (9 tests), all passing on Windows/MSVC:

- Discovery lists `workspace_fs` (5 actions) and `workspace_shell` (`RunShell`, argv note).
- Default-deny shell; explicit `cargo --version` grant succeeds.
- Unknown executables and path forms (`a/b`, `a\b`, `a:b`, empty) denied.
- Argument-prefix grants allow `--version` while denying `--help`.
- Nonzero exit (`cargo --nonexistent-flag-xyz`) reported with bounded evidence.
- Excess output (cmd `for /L` loop, ~200 KiB) fails with exceeded notice; evidence < 256 KiB.
- Timeout (`ping -n 30`, 2 s limit) returns promptly with "timed out" and the
  supervisor stays usable for the next call.
- Tree-kill test returns promptly instead of hanging for the full ping duration.
- Environment cleared: secret absent without a grant, forwarded with `allow_env`
  (Unix assertion; Windows asserts the cleared case).

Full suite after this change: 47 passing (38 prior + 9 new). `cargo fmt --check`,
`cargo check --locked`, `cargo test --locked`, and `cargo clippy -- -D warnings` pass.
`file-core-v1` benchmark unchanged: 10/10 positive, 10/10 negative, 30 tool calls,
40 model decisions, ~0.49 s single Windows debug sample.

## Limits (honest)

- Authorization is not isolation: no job objects, cgroups, namespaces, or sandbox.
  Concurrent hostile workspace mutation, hard links, and device/DLL-planting style
  attacks are outside the threat model. Use trusted isolated workspaces.
- Unix tree cleanup is best-effort (direct child only). Windows relies on `taskkill /T`,
  tested for prompt return, not a proven descendant census.
- No stdin interaction, no I/O deadlines below the wall-clock timeout, no CPU/memory caps.
- Shell actions still cannot be reconciled after interruption (refused, as before).
- The deterministic CLI model still only understands file-creation objectives; shell
  actions are available to typed model implementations through the runtime interface.
