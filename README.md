# Rust Agent Harness v0.1

A local-first autonomous-agent runtime research project. Milestone 1 implements a narrow but complete vertical slice:

`objective -> model decision -> permission check -> tool action -> observation -> verification -> append-only persistence`

## Current executable capability

The dependency-free milestone model understands:

```text
create file <relative-path> with content <text>
write file <relative-path> :: <text>
```

It writes the file inside a configured workspace, reads it back as independent evidence, verifies exact contents, and records each state transition/tool call in an append-only event log.

## Run

Requires Rust 1.98.1+.

```bash
cargo test --workspace
./scripts/demo.sh
```

## Security posture of milestone 1

- Filesystem actions are confined to relative paths under the selected workspace.
- `..`, absolute paths, and path prefixes are denied.
- Shell execution exists as a typed tool but is limited to a small executable allowlist.
- No root/admin operations, secrets, browser, GUI, network, package installation, or external side effects are granted.
- Execution success is not accepted as task success: the write action is followed by a read-back verification step.

See `docs/architecture-v0.1.md` for the complete design and roadmap.
