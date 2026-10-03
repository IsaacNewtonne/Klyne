# File-write progress recovery

The October 1 audit retained a two-output task that wrote correct files but
continued rewriting them until its budget expired. The existing completion
shortcut could not help: it required the model to mark every task Done first.

## Behavior

Before a worker repeats `write_file`, Studio checks the newest destination
receipt from the current goal and compares its digest with the proposed content.
Workspace-relative aliases such as `a.txt` and `./a.txt`, and supported absolute
paths within the selected workspace, use the existing path policy.

A matching historical receipt is only a candidate. Studio hashes the file again
through a read-only tool. If the current bytes differ or cannot be checked, the
write stays on its ordinary execution path. Changed-content revisions are not
suppressed.

When the current bytes match, Studio records `progress_check`, skips the write,
and asks the read-only reviewer to inspect the current task. A reviewer can
report that task complete or give concrete repair guidance. Guidance does not
replace the plan: outstanding tasks, dependencies, original goal, acceptance
contract, and shared budgets remain intact. Normal final review and host
acceptance still run. File equality alone never completes the task.

Two recovery reviews are allowed without intervening successful file progress;
a third repeated write blocks with unfinished work saved. Resume alone does not
reset this count. A newly successful write/patch receipt does. Each recovery
review is also bounded to eight model decisions, with ordinary step, token,
time, cancellation, and permission checks remaining active.

This controller does not replay or suppress shell, browser, desktop, API, or
uncertain pending actions. It does not detect every loop, such as repeated reads
or alternating file contents. Review still uses the selected model; this is not
an independent improvement in creative judgment or model training.

## Validation

Regression coverage includes repeated two-output completion, intervening reads,
missing-output repair, preserved dependencies, fresh disk changes, changed-content
revisions, reviewer write denial, failed acceptance, recovery exhaustion across
Resume, and recovery after actual progress. A unit fixture also checks path
aliases, missing/failed receipts, traversal, and non-file actions.

The first broader Studio run passed 102 unit tests (3 ignored) and 49 chat tests,
but failed six browser/desktop-dependent fixtures in the sandbox. The unrestricted
rerun passed all 102 unit, 56 chat, and 2 runtime tests (4 fixtures ignored),
including the six initial failures. After refining the recovery counter to reset
on actual file progress, the final targeted run passed its unit test and all
7 progress integration tests, including the newly added progress-reset case.
Strict all-target Studio Clippy, workspace formatting, and diff whitespace
checks passed. The runtime suite's printed crash-loop error is expected fixture
output, not an additional failure.

Commands:

```powershell
cargo test --locked -p klyne-studio --bin klyne-studio --test chat --test runtime -- --test-threads=1
cargo test --locked -p klyne-studio --bin klyne-studio --test chat progress -- --test-threads=1
cargo clippy --locked -p klyne-studio --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
python scripts/audit-live.py --model tiger-gemma-12b-v3:IQ4_XS --case multiple
```

A local smoke run used `tiger-gemma-12b-v3:IQ4_XS` with the existing 32-step,
180-second limits and the unchanged `multiple` case in `scripts/audit-live.py`.
It completed in 14 accounted steps, wrote each file once, and independently
passed both exact-content checks and source-preservation checks. Evidence:
`workspace/live-audit-1791013420481835800/` (including the binary hash).

That live run did **not** trigger `progress_check`, so it does not establish that
the controller caused the success or a latency gain over the historical failed
run. It also included an inaccurate `relative/path/` prefix in its final prose;
the real files were in the fixture workspace. Artifact success is not a claim
of perfect final-answer quality. The forced-repeat regression fixtures directly
exercise the new path.

No AgentJev service, model weights, training pipeline, or learned strategy bank
was installed by this change. This is the first execution-control increment
from the research assessment.
