# Tool autonomy upgrade

## Objective

Given an authorized task, Klyne should discover how to do it, inspect the relevant
state, perform the work, verify the result and interrupt only for a real user
decision or host-enforced limit. Unknown tools should enter discovery, not a
special-case list of exceptions.

## Findings

The existing runtime already has access controls, action journals, uncertainty
handling, independent file checks, browser/desktop observations, qualified custom
tools and supervised activation. The missing layer was a shared operating
contract. A worker could ask an unsupported question after one incomplete
observation. Recent-context trimming also made earlier tool output hard to recover.
Source builds did not update the versioned binary selected by the launcher.

## Implementation

1. **Shared operating contract.** Every Studio planner, worker and reviewer receives
   the discovery, inspection, prerequisite, action and verification protocol.
   It applies to file, terminal, browser, desktop, API, MCP, capability and runtime
   routes, including future routes using the same request loop.
   An explicit built-in inventory is supplied independently of the saved
   extension catalog. The live local model previously treated an empty extension
   catalog and disabled Terminal access as disabling workspace file tools. File
   tools now appear explicitly even when all external access flags are off.
   Worker-specific instructions distinguish executing a step from planning it.
2. **Supported interruptions.** Questions carry a structured blocker: kind,
   missing information, why it needs the user, and evidence references for
   environment blockers. The host checks reference existence and checks supplied
   user text for the claimed missing value. Unsupported questions get bounded
   correction before being surfaced. Repeated unsupported responses fail as a
   model limitation rather than becoming an invented user prerequisite.
3. **Recovery context.** Up to 24 current-goal action summaries retain absolute
   evidence IDs in model context. `evidence_read` retrieves historical output in
   pages without executing the original operation again. Historical evidence
   cannot substitute for fresh completion verification.
4. **Future tool contracts.** Registered tools can describe prerequisites,
   discovery, verification and failure recovery. Validation bounds the metadata
   and excludes permission fields. The metadata is returned during capability
   discovery, is versioned with the tool, and is advisory. Existing qualification,
   access and effect rules remain authoritative. Legacy tools remain usable.
5. **Activation.** `--stage-runtime ROOT BINARY TEST_WORKSPACE -- PROGRAM ARGS...`
   runs the supplied test command through the host runner, binds passing output
   to unchanged binary bytes, preflights and queues the candidate. The supervisor
   activates only when idle and retains its startup rollback path. Staging is
   not proof of activation; inspect `runtime/last-result.json` and `current.json`.
6. **Build prerequisites.** Shell execution, custom tools and runtime attestations
   share an explicit local toolchain environment, including Rust selection and
   Windows SDK/linker variables. Secret environment variables remain excluded.

## Tool family expectations

| Family | Discovery and inspection | Verification and recovery |
|---|---|---|
| Files | List/search/stat, read relevant content | Read-back/digest; reconcile partial writes |
| Terminal | Inspect project instructions and command help | Inspect output and actual artifacts; nonzero exit is not rollback |
| Browser | Inspect session, page and schema/controls | Verify destination and visible result; do not duplicate submissions |
| Desktop | Observe, focus the target, inspect menus/controls | Fresh observation after input; respect Stop and uncertain effects |
| App APIs | Discover connection and operation schema | Inspect concrete destination; distinguish connection failure from uncertain dispatch |
| MCP | Discover tool schemas and inspect resources | Treat annotations as advisory; use existing host policy |
| Custom tools | Read operating contract, qualify executable/arguments | Verify effects separately from a successful process exit |
| Runtime | Read status, build and test candidate | Attested preflight, supervisor health and rollback evidence |

## Acceptance and limits

Regression fixtures cover unsupported planner and worker questions recovering
into an actual directory inspection, bounded repeated-question failure, real
preferences remaining answerable, authentication with evidence, invented evidence
IDs, reuse of supplied information, historical evidence pagination, and future
tool metadata. Existing suites exercise route permissions, uncertain effects,
restart, completion verification, browser operations and custom tools.

Evidence-reference validity is host-checked; whether evidence semantically proves
ambiguity still involves model judgment. A small model can still choose poor
actions or produce a well-formed but mistaken blocker. This change does not add
every conceivable adapter, certify third-party tools, or establish a general
unattended success rate. The core CLI runtime is separate from Studio's model
request loop and does not acquire the new Studio decision schema automatically.

## Next architectural work

- Independently evaluate blocker semantics against observations, with measured
  false-interruption and incorrect-continuation rates.
- Add native browser tab/frame/dialog and transfer contracts where required.
- Extend semantic target resolution and test representative multi-monitor,
  profile-menu and modal workflows with real providers.
- Establish a fixed live task corpus across tool families; measure completed
  outcomes, user interventions, duplicate effects, latency and model cost.
- Add richer durable subprocess sessions and resource-specific verification
  adapters where current process/output contracts are insufficient.

These are separate capabilities, not claims established by prompt changes or
passing deterministic fixtures.

## Validation recorded September 16, 2026

- Candidate SHA-256: `c370fc2e877fd435e7295081ce2887ab733bf029cb04d85c7a0d9a96562ea984`.
- Direct regression gate: **131 passed** (85 Studio unit, 39 execution, 5 security,
  2 supervisor); 4 intentionally ignored subprocess/live-MCP fixtures.
- Two existing visual UI assertions were excluded from this gate after failing
  in the broader run: `chat_browser_conversation_settings_controls_and_mobile`
  and `production_view_tracks_real_model_workers_evidence_and_result`. This is
  not a claim that the entire UI suite passes.
- Live configured Ollama model: `hf.co/empero-ai/Qwythos-9B-Claude-Mythos-5-1M-GGUF:Q5_K_M`.
  It completed read-only workspace inspection using `list_dir:.` with all external
  access switches off. A malformed initial planning response recovered automatically;
  no clarification was requested. Record: `workspace/autonomy-live-result.json`.
- An earlier live run exposed the empty-extension-catalog misconception described
  above. The final run used the corrected built-in inventory and worker instructions.
- Activation attestation reruns the six pure autonomy unit checks against the
  unchanged candidate digest, after the complete direct gate above. Running the
  whole browser/restart suite inside the nested process runner previously retained
  inherited output handles until its timeout; no passing attestation was issued
  for that timed-out run.
- Live Chrome profile navigation, WhatsApp delivery and general unattended task
  success remain unmeasured by this smoke test.
- Activation confirmed: `runtime/current.json` and `runtime/last-result.json`
  both select the candidate hash above, `rollback:false`; health reports the new
  PID `50696` with `ready:true`. The temporary smoke server and model-trace proxy
  were stopped after verification.
