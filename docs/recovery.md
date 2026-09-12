# Independent recovery

Klyne's recovery service runs outside Studio and outside the model-driven chat
loop. Its page defaults to http://127.0.0.1:4318; Studio stays on port 4317.
The Recovery link opens that separate page. A crash or broken Studio frontend
does not take the recovery page down.

## Start

Build both binaries, stop the existing Studio instance, then run from this repo:

```powershell
cargo build -p klyne-studio --bins
python scripts/klyne_recovery.py
```

Options include `--root`, `--source`, `--binary`, `--supervisor`, `--port`, and
`--recovery-port`. Python uses only its standard library. The guardian launches
the Rust supervisor, which owns Studio's activation and rollback protocol.
Duplicate recovery listeners and existing Studio listeners are refused.

## Configure model recovery

Create `workspace/studio/recovery/config.json` using an installed local model:

```json
{
  "enabled": true,
  "fallback": {"kind":"ollama","endpoint":"http://127.0.0.1:11434","model":"your-installed-model"},
  "repair_provider": {"kind":"ollama","endpoint":"http://127.0.0.1:11434","model":"your-installed-model"},
  "automatic_code_repair": true
}
```

Repair is automatic: no Recovery button press is needed. The separate page is
for status and Stop. Local repair uses Ollama's native chat API, constrained JSON
output, disabled thinking, and on-demand screenshot input for a vision-capable model.
The local agent receives focused source excerpts in an 8K context budget; it can
request additional sections or the saved image when needed.
It does not call Codex or silently switch to a cloud provider. If the local repair
server/model is unavailable before diagnosis starts, queued incidents wait and
availability is checked again every thirty seconds. The subprocess deadline and
Stop button also bound local inference. A model failing during a repair leaves
its evidence and failure report; it is not an unlimited retry loop.

The same installed model can serve chat and repair, but both depend on that
Ollama server. A separately configured local server/model gives better isolation.
A working connection cannot guarantee a correct repair; patches still pass the
independent gates below.

Codex remains an explicit optional configuration: set `repair_provider` to
`{"kind":"codex"}` and set `fallback` similarly to use an authenticated Codex
CLI. Local mode never selects it automatically. The repair agent returns
structured inspection requests and exact patches; the host validates paths and
patch context before performing those narrow operations. The model does not
receive shell authority. For Codex, `repair_model` optionally selects its model.

For watchdog and evidence collection without model access, use
`{"enabled":true,"automatic_code_repair":false}`. No cloud calls are made by
that configuration. With no config, Studio preserves its prior behavior.

## Recovery behavior

* A failed model request can try the configured fallback once. The saved primary
  provider and original task permissions remain unchanged. Recovery appears in
  the conversation. Failure of both connections produces a durable incident.
* Incidents preserve the request context, role, bounded model response excerpts,
  errors, and checkpoint identity. These files may contain conversation content;
  a configured cloud repair model receives the incident and source snapshot.
* The guardian captures only Klyne in an isolated headless browser. It does not
  capture a personal browser profile or the full desktop. Browser failure is
  recorded and does not prevent diagnosis from other evidence.
* Model-driven repair copies the current dirty/untracked source without modifying
  the original checkout. Live databases, credentials and generated workspaces are
  excluded. The agent writes a diagnosis and plan before repairing the copy.
* Fixed baseline and candidate Studio tests run independently of the agent.
  Existing integration tests, manifests, supervisor, activation code and recovery
  bridge are protected from repair edits. Changes are limited to 16 Studio source,
  web and new integration-test files. This is source isolation, not a virtual
  machine; the host's normal process and CLI sandbox restrictions still apply.
* Passing candidates are built and started in a separate data directory and port.
  A real, tool-free greeting smoke test must complete using the primary provider.
  This smoke test checks the model conversation path, not every possible user task.
* The supervisor stages the candidate and retains the previous binary. A failed
  candidate startup rolls back. A still-blocked, unchanged conversation resumes
  only if it has no pending action with an uncertain outcome. The system reports
  "resumed", not "fixed", until the resumed work supplies its own result.
* A crash or five failed health probes triggers a restart. Three starts per
  fifteen minutes is the limit, persisted across guardian restarts. A conversation
  gets at most two automatic code-repair attempts. Incidents are never replayed
  just because the guardian restarts.

The recovery page's Stop button cancels active repair subprocesses and persists
the pause in `recovery/paused.json`; runtime monitoring continues. To re-enable
repairs, remove that explicit pause file and restart the recovery service after
inspecting the report. A candidate already handed to the supervisor may finish
activation; Stop will not replay the original conversation afterward.

## Evidence and limits

`recovery/status.json`, `recovery/runtime.log`, `recovery/incidents/` and
`recovery/reports/` preserve progress independently of the app. Reports include
the screenshot result, agent output, baseline/candidate test logs, build and
smoke-test results, source changes and activation result.

This is bounded recovery, not a guarantee of fixing every failure. An offline
machine, unavailable model/login, full disk, missing compiler, failures in the
guardian itself, or unknown side effects can require intervention. Runtime
crashes are restarted; automatic source repair is currently triggered by exhausted
model-request failures. Arbitrary task failures and permission denials are not
treated as permission to rewrite the app. Functional regressions after successful
startup are not automatically rolled back. They remain visible in the resumed
conversation and recovery evidence.

## Verification

```powershell
cargo test -p klyne-studio --bins --test chat --test runtime -- --test-threads=1
$env:KLYNE_TEST_BINARY = (Resolve-Path target/debug/klyne-studio.exe).Path
python -m unittest discover -s scripts -p test_klyne_recovery.py -v
```

Tests cover provider failover without changing the saved primary, bounded failure
recording, uncertain/stopped/changed replay refusal, source snapshot preservation,
protected gates, process deadlines and cancellation, persistent restart limits,
and a real runtime crash while the independent recovery page stays reachable.

`python scripts/recovery_drill.py --local` runs the end-to-end drill using the
first installed Ollama model; omit `--local` only to explicitly test Codex. It injects a parser defect only into a temporary source copy,
reproduces the failed conversation, runs the independent repair service, and
requires the repaired runtime to complete that same conversation. The successful
drill on 2026-09-12 preserved its evidence in
`workspace/recovery-drill-1789146708/verified.json` and the adjacent recovery
reports. The original checkout and normal Studio database were not modified by
the drill.

Local validation on 2026-09-12: the installed Ollama model produced a parser
repair in the isolated source copy. The repaired candidate passed all 38 Studio
checks and the previously failing response path completed. Local proposal records
are in `workspace/local-agent-check-3/`; the independent smoke result is
`workspace/local-repair-verified/smoke-result.json`. The normal workspace now
configures both request fallback and automatic code repair to Ollama, without
a cloud fallback.
