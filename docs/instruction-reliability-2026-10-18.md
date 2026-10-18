# Instruction reliability changes (Windows checkout)

Implemented locally from base `b1cb85b` on 2026-10-18. This is a fresh
implementation based on the supplied report, not an application of its missing
patch or a verification of its claimed Linux logs.

## Changes

- Document extraction failures become persisted failed observations, allowing
  the worker to inspect alternatives without resubmission. Tool timing and step
  accounting are retained; cancellation and budget guards still apply.
- Stall history retains 24 hashed operation/outcome pairs, enough for three
  observations in cycles of up to eight operations. Document reads and saved
  capability discovery participate. Changed outcomes are not treated as stalls.
- Memories record host time/workspace and Studio conversation/goal/evidence
  count. Their status is `model_reported`, regardless of a model's claimed
  verification. Legacy notes without metadata are `legacy_unverified`.
- Saving the same memory name creates a correction version. `memory_forget`
  transactionally deletes every version and its active entry, preventing restore.
  Terminal and worker permissions are required; reviewers cannot mutate memory.
- Known granted values are scrubbed before capability saves reach SQLite.
  New memory content with conservative credential markers is refused; legacy
  sensitive notes remain excluded from automatic recall. This is not complete
  sensitive-data detection or secure erasure of old database bytes/backups.
- Unix subprocess setup uses the required unsafe boundary, documents its safety
  assumptions, and fails spawn if `setsid` fails. Windows-only tests and locals
  use compilation gates.

## Validation performed

Windows, Rust 1.98.1, MSVC. Scripted model fixtures and real host tools; no live
language-model evaluation or Linux execution was performed.

- Studio unit suite: 110 passed, 3 ignored.
- Selected Studio chat suite: initial run 61 passed, 5 failed, 3 filtered.
  One failure was a new test fixture using the wrong DB location; after fixing
  that and matching the caller contract goal to its instruction, the new memory
  integration passed. The other four tests involved browser/desktop access and
  all passed when rerun outside the restricted execution sandbox. Thus all 66
  selected tests passed across the run and targeted reruns, not one clean run.
- New regressions cover missing-document recovery in one submission, persisted
  multi-operation cycles, memory correction/removal and false verification
  claims, credential-marker refusal and legacy filtering, and granted-value
  scrubbing followed by cross-conversation recall and verified CSV creation.
- Workspace suite excluding Studio/browser completed with failures only in
  delegation/process tests whose child Cargo could not read the user's Rust
  configuration inside the sandbox. Outside the sandbox, delegation passed
  13/13 and process passed 12/12 with 3 subprocess fixtures ignored, including
  Windows tree termination.
- Python recovery: 13 passed, 1 skipped.
- Strict workspace/all-target Clippy, formatting and diff whitespace checks pass.

Commands used (targeted reruns used the named regression filters):

```powershell
cargo test --locked -p klyne-studio --bin klyne-studio -- --test-threads=1
cargo test --locked -p klyne-studio --test chat -- --test-threads=1 --skip production_view --skip chat_browser --skip recovery_screenshot
cargo test --locked --workspace --exclude klyne-studio --exclude harness-browser --no-fail-fast -- --test-threads=1
cargo test --locked -p harness-core --test process --test delegation -- --test-threads=1
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
python -m unittest discover -s scripts -p test_klyne_recovery.py
```

## Limits and deployment

The three explicitly filtered browser/UI cases were not run. Passing scripted
fixtures does not prove general task understanding, subjective review quality,
or live prompt-injection resistance. Longer or semantically varied loops still
depend on budgets. Final-save failure recovery, broader acceptance contracts,
action-specific approvals, OS isolation and upload/download contracts remain
separate work.

Changes are in the checkout; no GitHub push or installed-runtime activation was
performed. See `voice.md#updating-an-existing-runtime` for separately activated
runtimes. Memory metadata is additive; code rollback does not restore forgotten
memories. Tests used disposable fixture data, not real user memories.
