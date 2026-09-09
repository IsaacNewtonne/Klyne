# Rust Agent Harness

A local-first Rust runtime prototype. **MODEL != AGENT**: models propose actions; the runtime owns permissions, evidence, budgets, and persistence.

## IMPLEMENTED and tested

- Two-crate workspace: harness-core and harness-cli.
- Replaceable model trait and deterministic file-objective adapter.
- Typed file actions, permission checks, independent runtime read-back verification.
- Serializable success criteria and a verifier trait independent of the model adapter.
- SQLite migrations, versioned events, transactional checkpoints, process-exclusion lock.
- Durable objective, history, step budget, pending action, and terminal outcome.
- Resume between actions after process termination; refuse uncertain interrupted actions.
- Negative security tests, actual process-kill recovery test, checkpoint-boundary tests.

## Run

Requires pinned Rust 1.98.1 and a native C compiler for bundled SQLite.

```sh
cargo run -p harness-cli -- --workspace workspace/example --objective "create file hello.txt with content hello agent"
cargo run -p harness-cli -- --workspace workspace/example --resume
cargo run -p harness-cli -- --workspace workspace/example --events
```

One run per database, defaulting to `<workspace>/.harness/run.sqlite3`. Use a fresh workspace or explicit `--database <path>` for another run. Terminal resume returns the historical result without new actions or fresh verification. The legacy text store remains available to library callers without recovery support.

The adapter accepts `create file <relative-path> with content <text>` or `write file <relative-path> :: <text>`. It is not an LLM and cannot solve general coding tasks.

## Verify and benchmark

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run -p harness-core --example benchmark --locked
```

The benchmark reports JSON and fails if any of 10 file tasks or 10 traversal cases violate acceptance criteria. It has no external model calls, tokens, or model cost.

## PARTIAL / limitations

- Path checks reject traversal, reserved runtime paths, existing symlinks and Windows reparse points. They are **not an OS sandbox**: concurrent path replacement, hard links and hostile workspace mutation remain unresolved. Use trusted isolated workspaces. Writes are not atomic.
- Shell source is retained, but default policy denies execution. Argument scopes, timeouts, output bounds and isolation must precede enabling it.
- Checkpoints are authoritative; events and checkpoints are not one transaction across an entire tool call. Uncertain actions need reconciliation. No exactly-once or host-power-loss guarantee.
- Recovery assumes a stateless model. One database handles one run. Only cognitive-step budgets exist.
- Objectives, observations and checkpoints contain task data. Secret redaction is absent: do not supply secrets.
- Goal DAGs, planners, real providers, general coding, memory, browser, GUI, delegation and self-improvement are **PLANNED**.

See [implementation evidence](docs/milestone-durable-core.md) and [historical architecture](docs/architecture-v0.1.md).
