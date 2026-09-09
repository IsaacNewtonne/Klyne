# First autonomous coding benchmark

Status: **IMPLEMENTED** as three scripted repair tasks with an independent
compiler/test oracle. General coding intelligence is **not** claimed: the
agent is a deterministic script, and what is benchmarked is the runtime
boundary (inspection → patch → build → test → repair → restart), not model
quality.

## What changed

- New `harness-benchmark` crate. Each task is a disposable dependency-free
  Rust crate with one injected bug (`fn answer()` returning the wrong
  constant) plus a failing unit test asserting the fixed value.
- The scripted repair agent works only through runtime tools: `SearchFile`
  inspection, `ReadFileRange` span confirmation, `HashFile` digests,
  digest-guarded `PatchFile`, and `cargo test` through the supervised process
  tool under a narrow grant (argv must start with `test`, 300 s timeout).
  Its decisions are history-driven, so it resumes correctly from any
  persisted prefix. Test-failure observations drive a re-hash → re-patch →
  re-test repair round; stale patch failures earn one re-hash.
- The independent oracle runs `cargo test --offline` via plain
  `std::process::Command`, outside agent control, and requires exit success
  plus `test result: ok`. Runtime `FileRange` evidence over the fixed span is
  a second, separate gate. A task counts only if the agent completes, the
  range evidence passes, and the oracle passes, with zero permission denials.
- Reports record tool calls (persisted budget), model calls (cognitive
  steps), test runs, failed-observation retries, denials, wall-clock seconds,
  tokens, and cost (0: scripted, no model spend).
- `run_task` resumes: a non-terminal checkpoint continues with the same
  history-driven model (missing success claims are repaired via an audited
  `set_success_criterion`), and terminal checkpoints report through the
  oracle without re-running. Kill mid-flight therefore continues rather
  than restarts; exactly-once patching is asserted in tests.
- Fixtures pin `rust-toolchain.toml` to 1.98.1 at the workspace root because
  rustup resolves the toolchain from the process working directory, and the
  machine default toolchain is broken. Fixture fixes are same-length so byte
  offsets stay stable from search to final verification (harness-asserted).
- Build environment: the cleared child env plus OS minimum cannot link MSVC
  (`link.exe` discovery fails). The benchmark grants an explicit documented
  set (`TEMP`, `TMP`, `USERPROFILE`, `ProgramFiles`, `ProgramFiles(x86)`,
  `ProgramData`, `SystemDrive`, `OS`, `NUMBER_OF_PROCESSORS`,
  `COMPUTERNAME`). Bisection showed the mechanism is redundant — no single
  variable is load-bearing — so the set is conservative, not minimal. See
  `BUILD_ENV_GRANTS`.

## Verified

`crates/harness-benchmark/tests/autonomous.rs`, Windows/MSVC, real `cargo`:

- `repairs_wrong_constant`: inspect → patch → test → verified (6 tool calls).
- `first_patch_fails_then_failure_driven_repair`: a plausible-but-wrong first
  patch fails its test, the agent re-hashes, patches to the final value, and
  re-tests green (9 tool calls, 2 test runs, 1 retry, oracle-verified).
- `crash_restart_completes_task_without_duplicating_effects`: a worker
  process is killed after the patch checkpoint (same ready-file pattern as
  the core kill tests); resume tests, verifies, and completes with exactly
  one patch in history.
- Sample report: `plausible-but-wrong`, completed, verified, 9 tool calls,
  9 model calls, 2 test runs, 1 retry, 0 denials, ~1.4 s, 0 tokens, $0.

Full suite after this change: 68 passing (64 prior + 4 benchmark).
`cargo fmt --check`, `cargo check --locked`, `cargo test --locked`, and
`cargo clippy -- -D warnings` pass.

## Limits (honest)

- The "agent" is a hardcoded strategy for one bug shape; it demonstrates the
  loop, not autonomy. Failure parsing is substring matching on cargo output.
- Three single-constant tasks in one file each: no multi-file reasoning,
  no dependency management, no genuinely novel bugs.
- The oracle and the agent run the same `cargo test`; correlated blindness
  (both missing a deeper bug the fixture test doesn't cover) is possible.
- MSVC build-env grants are conservative, not minimal, and Windows-specific
  in effect; Unix behavior of that set is untested.
- No wall-clock/token/cost budgets yet; the 300 s process timeout is the
  only backstop against a hung build.
