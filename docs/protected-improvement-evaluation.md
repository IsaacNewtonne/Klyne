# Protected improvement evaluation and experience

The `self_improve` route now separates frozen regression checks from evidence of
a specific benefit. Passing candidate-authored tests alone never sets
`improvement_verified` to true. A retained branch is a reviewable candidate, not
an automatically activated upgrade or proof of improved general intelligence.

## Regression reference

Before mutation, the runner snapshots and hashes tracked integration-test files,
Cargo manifests, lockfiles, build scripts, toolchain selection and Cargo
configuration. Rust syntax is parsed to retain baseline test functions, test
modules, relevant attributes and macro invocations. Changes to those definitions
are rejected even if the candidate prints the same number of passing tests.
The runner verifies the reference before and after candidate execution and
requires baseline named passing tests to remain passing in the candidate.

Additional candidate tests remain useful regression coverage, but must be placed
outside existing frozen test modules/files. Test or build-configuration revisions
require a separately reviewed baseline update. Syntax that cannot be parsed fails
closed. This deliberately restricts some legitimate refactors and dependency
changes within automated experiments.

## Independent acceptance

Library callers can configure `ExperimentRunner::with_acceptance_test(path)`
with a host-authored Rust integration-test source. Studio looks for that source
at `<studio-root>/acceptance/<repository-key>.rs`. The repository key is the
SHA-256 of the canonical absolute repository path, lowercased on Windows. The
candidate action cannot supply or override the acceptance source.

The runner freezes the source bytes before mutation and executes the same test
against baseline and candidate in the disposable worktree. It reserves the
`tests/__klyne_acceptance.rs` path and removes the injected source before retaining
the candidate branch. A missing host configuration is reported by null acceptance
results, with `improvement_verified: false`.

The current acceptance adapter supports a Cargo package with an integration-test
target. Tests may exercise an existing public library interface or executable.
It does not yet select individual packages in virtual workspaces or provide
latency/cost threshold benchmarking.

A specific improvement is marked verified only when the host test actually runs
on both versions, the baseline has a named failing test, every candidate
acceptance test passes, and the frozen regression gates also pass. Compilation
failures, missing tests, altered acceptance sources, timeouts and Stop do not
establish that gain. Baseline and candidate receive the same command and timeout
per suite; this is a bounded behavioral comparison, not a statistical estimate of
future task success.

## Experience records

Completed experiment records include the baseline commit, frozen reference hash,
candidate patch and hash, named test outcomes, output hashes and bounded output
tails, optional acceptance results, decision and timestamp. Rejected patches are
retained in the report even when their worktree is removed. Reusing a record ID
cannot overwrite earlier evidence. Failures before a report can be constructed
remain ordinary tool errors rather than complete experiment records.

Studio indexes completed records in `<studio-root>/conversations/experiences.sqlite3`.
Workers receive up to three keyword-matching historical records for their current
repository, selected from the most recent 50. The source record must still match
its indexed digest. Both rejected and retained experiments are eligible; neither
is injected as an instruction or a permission grant. Applicability to changed
code and environments must be checked again. Raw patches are retained for explicit
inspection rather than automatically placed into every worker prompt.

These are same-user integrity and orchestration controls, not an OS security
boundary against hostile candidate code. Cargo can execute build scripts and
tests with host authority; output parsing is not tamper-proof measurement. The
fixed checks also cannot detect every form of test overfitting or every indirect
test dependency. Runtime attestation/activation remains a separate route.

## Validation

Targeted tests cover same-count assertion weakening, manifest-based test
disabling, candidate tests without a benefit claim, a host-defined failing-to-
passing behavior, absence of the injected test from the retained commit, and
repository-scoped experience recall that rejects modified records.
Logs: `workspace/protected-experiment-tests-final.log`,
`workspace/protected-improvement-tests.log`, and
`workspace/protected-evaluation-workspace-tests.log`.

Validation on this machine: the full mixed workspace run reported **277 passed,
one failed, 10 ignored**. The failure exposed a Windows nonblocking-socket race
in the existing secret-binding fixture. After correcting that fixture, the whole
Studio chat target passed **34/34**, covering the previously failing test and its
neighbors (`workspace/protected-evaluation-chat-recheck.log`). All **278 unique
tests** are therefore covered by passing results; this was not a uniformly green
first run. Workspace Clippy with all targets/features and warnings denied,
formatting, and diff whitespace checks also pass. No live-model improvement or
cross-task performance gain has been measured.
