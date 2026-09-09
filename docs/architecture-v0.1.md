# System Architecture v0.1

> Historical proposal from the bootstrap commit. Its environment and milestone status describe that earlier session. See [current implementation evidence](milestone-durable-core.md) and [README](../README.md) for verified implementation and limitations.

Status legend: **IMPLEMENTED**, **PARTIAL**, **EXPERIMENTAL**, **PLANNED**, **BLOCKED**.

## 0. Environment baseline

Observed build environment on 2026-09-09:

- Linux x86_64, kernel 6.18.35.
- Git, GCC, Clang, CMake, pkg-config and Python are present.
- Rust/Cargo are absent.
- Docker/Podman/SQLite CLI were not found in PATH.
- Network bootstrap of rustup failed and apt metadata refresh timed out.
- Current stable Rust is 1.98.1; this workspace pins `rust-toolchain.toml` to 1.98.1.

Therefore the source tree is built to be dependency-free for milestone 1, but compilation in this container is **BLOCKED** by the missing Rust toolchain. No compile/test success is claimed.

## 1. Architectural principles

1. **Model != Agent.** Models produce bounded cognitive decisions; runtime, state, tools, memory, permissions and verification create agent behavior.
2. **Event-sourced execution.** Durable events are the source of truth for replay, auditing and recovery; snapshots accelerate restart.
3. **Capability security.** Tools do not imply authority. Each action must carry required capabilities and be policy-evaluated at execution time.
4. **Evidence before completion.** A successful tool call is an observation, not proof of success. Completion requires explicit verification evidence.
5. **Replaceable cognition.** Planning/search/reflection strategies implement interfaces and can be benchmarked against the same tasks.
6. **Local-first, remote-optional.** Core state and orchestration remain local. Remote models/services are adapters.
7. **Small composable primitives.** The system should prefer typed actions, observations, events, memories, goals and artifacts over workflow-specific branching.
8. **Controlled self-improvement.** Self-changes occur in isolated Git branches/worktrees, benchmarked against a baseline and promoted only after policy and regression gates.

## 2. Target system architecture

```text
                                +----------------------+
                                |      CLI / TUI       |
                                +----------+-----------+
                                           |
                                +----------v-----------+
                                |     Agent Runtime    |
                                | lifecycle/budgets    |
                                | checkpoints/recovery |
                                +---+----+----+----+----+
                                    |    |    |    |
                    +---------------+    |    |    +----------------+
                    |                    |    |                     |
             +------v------+      +------v----v------+       +------v-------+
             | Cognition   |      | Goal/Plan Graph |       | Event Bus /  |
             | Strategy    |      | Scheduler       |       | Event Store  |
             +------+------+      +------+----------+       +------+-------+
                    |                    |                         |
          +---------+--------+    +------+--------+        +-------v--------+
          | Context Engine   |    | World State   |        | Observability  |
          +----+----------+--+    +------+--------+        +----------------+
               |          |              |
       +-------v--+   +---v---------+    |
       | Memory   |   | Model Router|    |
       +----------+   +-------------+    |
                                          |
                       +------------------v------------------+
                       | Typed Tool / Capability Registry    |
                       +---+----------+----------+------------+
                           |          |          |
                    +------v--+  +----v----+ +---v---------+
                    | Files   |  | Shell   | | Browser/GUI |
                    +---------+  +---------+ +-------------+
                           \          |          /
                            +---------v---------+
                            | Permission Engine |
                            +---------+---------+
                                      |
                            +---------v---------+
                            | Sandbox / Host OS |
                            +-------------------+
```

### Core data types

- `Goal`: desired state, success criteria, constraints, priority, deadline, budget.
- `Task`: executable node derived from goals and dependencies.
- `Action`: typed proposed side effect.
- `Observation`: typed result from environment/tool invocation.
- `Evidence`: verification artifact supporting a claim.
- `Belief`: world-state proposition with status `{Known, Believed, Unknown, Stale}`, confidence, source and last-observed time.
- `Memory`: structured durable item with type, salience, provenance, links and retention policy.
- `Event`: immutable execution fact.
- `Checkpoint`: restart snapshot that references the last committed event sequence.

## 3. Proposed Rust workspace

The first milestone intentionally implements only `harness-core` + `harness-cli`. The target decomposition is:

```text
crates/
  protocol/          shared IDs, schemas, Action/Observation/Event types
  runtime/           lifecycle, scheduling, cancellation, watchdogs, recovery
  cognition/         cognitive strategy trait + OODA/reflective implementations
  goals/             goal DAG, constraints, dependencies, success criteria
  planning/          hierarchical planning, repair, search, uncertainty/cost
  tools/             tool traits, registry, semantic capability discovery
  permissions/       capability tokens, policy evaluator, approval protocol
  sandbox/           process/container/VM isolation backends
  memory/            layered memory API, consolidation and retention
  context/           retrieval/ranking/context packing
  models/            provider trait, router, usage/cost accounting
  computer/          filesystem, shell, processes, accessibility abstraction
  browser/           CDP/WebDriver bridge and browser state
  verification/      evidence claims, validators, independent checks
  events/            durable append-only log + subscriptions
  observability/     tracing, metrics, structured run inspection
  secrets/           keychain/encrypted-store adapters and scoped leases
  agents/            parent/child delegation, budgets, shared artifacts
  benchmarks/        benchmark protocol, runners, result schema
apps/
  cli/
  daemon/
  tui/
  dashboard/         later Axum/web UI
experiments/
  tree-search/
  actor-critic/
  debate/
  memory-policy/
docs/
tests/
```

Rule: split a crate only when it defines a stable ownership or security boundary; avoid a crate-per-concept explosion.

## 4. Core trait/interface definitions

Conceptual interfaces; milestone-1 equivalents exist in `harness-core`.

```rust
trait ModelProvider {
    async fn infer(&self, req: ModelRequest) -> Result<ModelResponse>;
    fn capabilities(&self) -> ModelCapabilities;
}

trait CognitiveStrategy {
    async fn next(&mut self, state: CognitiveState, ctx: ContextBundle)
        -> Result<CognitiveDecision>;
}

trait Tool {
    fn descriptor(&self) -> ToolDescriptor;
    async fn execute(&self, ctx: ToolContext, input: Value) -> Result<ToolResult>;
}

trait PermissionEvaluator {
    async fn evaluate(&self, principal: Principal, request: CapabilityRequest)
        -> PermissionDecision;
}

trait MemoryStore {
    async fn write(&self, memory: MemoryRecord) -> Result<MemoryId>;
    async fn retrieve(&self, query: MemoryQuery) -> Result<Vec<ScoredMemory>>;
    async fn update(&self, patch: MemoryPatch) -> Result<()>;
    async fn forget(&self, policy: ForgetPolicy) -> Result<ForgetReport>;
}

trait Verifier {
    async fn verify(&self, claim: Claim, evidence: EvidenceSet) -> VerificationResult;
}

trait EventStore {
    async fn append(&self, event: Event) -> Result<EventSeq>;
    async fn stream_from(&self, seq: EventSeq) -> Result<EventStream>;
}

trait Sandbox {
    async fn spawn(&self, spec: SandboxSpec) -> Result<SandboxHandle>;
}
```

Design preference: serialized request/response schemas at process boundaries; strongly typed Rust internals; version every externally persisted protocol.

## 5. Threat and permission model

### Threat actors/failure modes

- Prompt/content injection attempting to trigger host actions.
- Model hallucination or planning mistakes.
- Malicious repository/tool output.
- Compromised tool/plugin/provider.
- Credential exfiltration through network or logs.
- Shell injection and path traversal.
- Confused-deputy privilege escalation.
- Symlink/race attacks around filesystem confinement.
- Runaway loops, resource exhaustion and fork bombs.
- Destructive self-modification.
- Cross-agent privilege leakage.

### Capability model

Every tool action declares capabilities such as:

```text
filesystem.read(path scope)
filesystem.write(path scope)
shell.execute(executable + cwd + env scope)
network.http(domain/method scope)
browser.control(profile/domain scope)
gui.control(app/window scope)
process.manage(pid/process-tree scope)
secrets.read(secret-id scope)
package.install(package-manager/repository scope)
admin.execute(operation scope)
external.publish(destination scope)
financial.transact(amount/merchant scope)
```

Policy result is one of `ALLOW`, `DENY`, `ASK`, `ALLOW_WITH_LIMITS`.

Limits can include max invocations, bytes, wall-clock duration, spend, domain/path/executable allowlists and expiration. Capability grants should be explicit objects/leases, not ambient booleans.

### Security invariants

1. Tool registration never grants authority.
2. A child agent receives the intersection of delegated capabilities and parent capabilities.
3. Secrets are referenced by ID and injected at execution boundaries; raw values should not enter general model context or event logs.
4. Privilege elevation is a separate audited transaction.
5. Host-destructive and externally irreversible actions are policy-gated even if an LLM requests them.
6. Sandbox escapes or policy failures fail closed.
7. Shell should eventually execute argv arrays rather than shell strings by default.

**Milestone 1: IMPLEMENTED** path traversal/absolute-path rejection, workspace confinement and shell executable allowlist. **PARTIAL** because symlink-safe `openat`/handle-based confinement is not yet implemented.

## 6. Memory architecture

Memory is a service with multiple record classes, not a chat transcript.

### Layers

- **Working memory:** ephemeral, task-local structured facts and scratch artifacts.
- **Episodic memory:** significant time-indexed events and outcomes.
- **Semantic memory:** normalized facts/entities/relationships with provenance and confidence.
- **Procedural memory:** reusable successful workflows, code snippets, tool sequences and preconditions.
- **Project memory:** repository architecture, conventions, decisions, active constraints and artifact map.
- **Failure memory:** attempted approach, failure signature, root cause and recovery.
- **User/environment memory:** stable non-secret preferences and environment capabilities/configuration.

### Storage plan

- SQLite first: source-of-truth metadata, links, events and FTS.
- Embeddings as a secondary retrieval index, not authoritative storage.
- Optional graph projection for entity/causal/dependency links; do not require a graph database initially.
- Content-addressed artifact store for large blobs and snapshots.
- PostgreSQL later for distributed/multi-host execution.

### Retrieval pipeline

`query generation -> hard filters -> lexical/vector candidates -> graph/link expansion -> temporal/salience ranking -> diversity -> token-cost packing`

Score dimensions: semantic relevance, goal relevance, recency, salience, provenance quality, failure similarity, project scope and estimated context cost.

### Consolidation/forgetting

- Merge duplicate facts and repeated episodes.
- Promote repeated successful episodes to procedural memory.
- Decay low-salience unreferenced memories.
- Preserve provenance and tombstones for safety-critical facts.
- Revalidate stale environment memories before acting on them.

## 7. Agent execution-loop design

Default cognitive strategy:

```text
OBSERVE
  -> ORIENT (world-state delta, uncertainty, budgets)
  -> RECALL (targeted memory/tool retrieval)
  -> PLAN / REPAIR
  -> SELECT ACTION
  -> AUTHORIZE
  -> ACT
  -> OBSERVE RESULT
  -> VERIFY CLAIMS
  -> REFLECT only on significant outcome/failure
  -> UPDATE world state / memory / plan / events
  -> CHECK termination, budgets, deadlock, cancellation
  -> CONTINUE
```

Important implementation details:

- The runtime owns termination/budget/cancellation semantics, not the model.
- Cognitive strategies emit typed `Decision`s rather than directly touching the host.
- Verification may enqueue new observations or independent agents.
- Reflection is trigger-based: repeated failure, surprising outcome, milestone completion or reusable discovery.
- Plan repair preserves useful completed subgoals rather than regenerating everything.
- Every loop iteration is bounded by token/tool/time/action budgets.

**Milestone 1: PARTIAL/IMPLEMENTED vertical subset** `orient/plan -> act -> observe -> verify -> complete`, with an 8-step runaway guard.

## 8. Computer-control architecture

Use a hierarchy from structured/high-confidence controls to visual fallback.

### Filesystem

- Rust-native API for metadata/read/write/patch/glob/watch/hash/diff/version.
- Capability scopes based on directory handles/canonical object identity rather than string prefixes.
- Atomic writes via temporary file + fsync + rename when needed.
- Content hashes in observations for verification and stale-state detection.

### Shell/process

- `tokio::process` supervisor with argv execution, streamed stdout/stderr, process groups/job objects, timeouts and cancellation.
- Resource limits via OS primitives (Linux cgroups/namespaces/rlimits; Windows Job Objects; macOS sandbox/process controls as feasible).
- Shell-string execution is an explicit higher-risk tool, separate from argv execution.

### Browser

Prefer CDP/WebDriver/Playwright-compatible bridge:

`DOM/accessibility tree -> network/console -> navigation/forms/downloads -> screenshot/vision fallback`.

Use isolated browser profiles and separate secret-bearing sessions.

### GUI

Backend trait with platform adapters:

- Windows UI Automation.
- macOS Accessibility API.
- Linux AT-SPI.
- Vision/input fallback only when structured accessibility is unavailable.

Action selectors should combine stable accessibility identifiers, geometry, app/window identity and visual verification.

## 9. Persistence/checkpoint strategy

### Source of truth

Append-only events with monotonic sequence numbers. Important writes must reach durable storage before externally visible completion is acknowledged.

### Snapshots

Periodic snapshot contains:

- active goals/tasks and DAG state
- cognitive strategy state
- model/tool budgets
- world-state beliefs and freshness
- outstanding approvals
- child-agent state
- memory cursors
- artifact references
- last committed event sequence

Recovery: load latest valid snapshot, replay events after its sequence, reconcile live external state, then resume only idempotent/retriable steps automatically.

### Idempotency

Every side-effecting action gets an operation ID. Tools that can support it use idempotency keys; otherwise the runtime records preconditions/postconditions and performs reconciliation before retry.

### Milestone 1

**IMPLEMENTED:** append-only event log persisted under `.harness/events.log` and flushed after each event.

**PLANNED:** binary/schema-versioned events, snapshots, checksums, fsync policy, action IDs and recovery replay.

## 10. Model-provider abstraction and router

A provider adapter exposes capabilities independently from branding:

```text
text_generation
structured_output
vision
reasoning_effort
tool_calling
context_window
streaming
local_execution
privacy_class
cost_model
latency_profile
```

`ModelRouter` chooses provider/model using task requirements, privacy constraints, quality history, estimated cost/latency and current rate limits. Routing itself should be benchmarked.

Provider classes:

- OpenAI-compatible HTTP APIs.
- Anthropic-compatible APIs.
- Gemini-compatible APIs.
- local Ollama/llama.cpp/vLLM/custom endpoints.

The harness normalizes requests/responses but stores provider-native metadata for debugging and usage accounting.

**Milestone 1: IMPLEMENTED interface, local deterministic adapter only.** Remote adapters are **PLANNED**.

## 11. Benchmark strategy

Benchmarks are versioned task suites with isolated environments, acceptance oracles and recorded budgets.

### Suites

- **Coding:** repository bugs/features; compile/test/static-analysis oracle.
- **Research:** source quality, factual correctness, citation coverage and unsupported-claim rate.
- **Computer use:** controlled desktop/browser tasks with final-state oracle.
- **Planning:** hidden dependencies, replanning after injected failures.
- **Memory:** relevant recall over long histories; distractor resistance; stale-memory correction.
- **Recovery:** process kill, host restart, tool timeout, corrupted intermediate state.
- **Permission safety:** prompt injection, path traversal, secret exfiltration, destructive-action probes.
- **Autonomy:** task distance achieved without intervention.

### Metrics

- completion/acceptance rate
- human interventions
- tool calls and redundant tool calls
- wall-clock/runtime
- model tokens and monetary cost
- retries/failures
- verification false-positive/false-negative rate
- permission violations prevented
- recovery success
- regressions versus baseline

Every self-improvement experiment produces a signed/immutable-ish record containing commit, configuration, model versions, benchmark version, random seeds where applicable, metrics and acceptance decision.

### First benchmark

`M1-file-create-verify`: given a workspace and objective, create requested file, independently read it back, verify exact bytes, persist execution events, reject `../` escape attempts.

## 12. Phased implementation roadmap

### Phase 1 — Safe local coding kernel

1. **IMPLEMENTED source:** runtime loop, model trait, file/shell tool traits, permission gate, event persistence, read-back verification.
2. Install Rust in an environment with network/toolchain access; run `cargo fmt`, `clippy`, tests and demo.
3. Replace milestone path checks with handle-based safe filesystem APIs and symlink defenses.
4. Add Tokio, structured serde protocol, `tracing`, cancellation tokens, timeouts and process supervision.
5. Add real OpenAI-compatible/local model adapter with strict structured decisions.
6. Build repository inspection + patch + build/test verifier loop.

### Phase 2 — Durable planning

Goal/task DAG, SQLite event/state store, checkpoints/replay, retries/idempotency, budget manager, planner/repair interface.

### Phase 3 — Memory/context

Layered memory schema, FTS/vector retrieval, context ranking/packing, consolidation and failure memory.

### Phase 4 — Browser

CDP bridge, isolated profiles, DOM/accessibility/network observations, download/artifact handling, browser benchmarks.

### Phase 5 — Delegation

Parent/child task contracts, capability/budget delegation, artifact mailbox, shared project memory, verifier agents.

### Phase 6 — Native GUI

OS accessibility backends first; screenshot/vision fallback; controlled desktop benchmark VMs.

### Phase 7 — Long-running autonomy

Daemon, watchdog, host restart recovery, deadlock/runaway detection, scheduling, TUI, resource limits.

### Phase 8 — Experimental cognition

Plug-in search/planning strategies: tree/graph search, debate, actor-critic, self-consistency, learned routing and world-model experiments.

### Phase 9 — Controlled self-improvement

Git worktrees, benchmark runner, safety/regression gate, automated retain/reject, rollback and experiment history.

## 13. Major unresolved research questions

1. What representation best connects symbolic goal/task graphs with flexible LLM reasoning without over-constraining it?
2. How should confidence and epistemic status propagate through world-state beliefs and model-generated observations?
3. Which context-retrieval policy maximizes task success per token rather than retrieval similarity?
4. When does multi-agent decomposition beat one stronger agent after accounting for coordination cost?
5. How can verification be made independent enough to catch correlated model errors?
6. What is the correct abstraction for GUI state across Windows/macOS/Linux while preserving semantic selectors?
7. How should long-running agents detect genuine deadlock versus productive slow progress?
8. Which memories deserve consolidation into procedures, and how are obsolete procedures invalidated safely?
9. How can permission prompts avoid both approval fatigue and excessive blocking while remaining understandable?
10. How should capability grants behave when tools call subprocesses or delegate to external automation frameworks?
11. What benchmark distribution best predicts broader autonomous competence instead of overfitting to coding tasks?
12. How should a self-improving harness separate improvements to orchestration from accidental exploitation of benchmark artifacts?
13. Can learned tool selection remain inspectable and policy-safe under thousands of dynamically discovered capabilities?
14. What replay semantics are required for external non-idempotent actions after crashes?
15. How should local/private model routing trade quality against privacy without leaking sensitive context to remote fallbacks?

## 14. Milestone 1 acceptance contract

Input:

```text
create file hello.txt with content hello autonomous harness
```

Expected runtime behavior:

1. Persist `GoalCreated`.
2. Local model emits typed `WriteFile` action.
3. Permission layer checks workspace confinement.
4. Filesystem tool writes the requested file.
5. Observation is persisted.
6. Model requests independent `ReadFile` verification.
7. Permission layer authorizes read.
8. Filesystem tool reads bytes back.
9. Model compares observed bytes with objective-derived expected bytes.
10. Only then persist `VerificationPassed` and `GoalCompleted`.

The negative test attempts `../escape.txt` and must fail before a write occurs.
