# Production view

## Experience

The same workspace changes from composing an instruction into observing its execution.
The submitted composer outline contracts into an AI core; the core then docks into
a live production graph. Worker cards and capability routes enter as real work is
reported. Completion settles the graph and returns emphasis to the answer and composer.

The interface must remain usable throughout. Stop, drafting, conversation switching,
evidence, and settings remain available. Animation never delays a request, hides an
error, creates a task, or implies success.

## State and transition contract

| Runtime signal | Presentation | Motion |
| --- | --- | --- |
| Submit in flight | Receiving instruction | Composer outline converges on the central core |
| Planning + model request | Thinking / planning | Core breathes; provider name and actual request role shown |
| Tasks available | Work assigned | Core docks left; worker cards unfold; assignment links connect |
| Worker model request | Thinking / working | Active worker and model route highlighted |
| Pending tool action | Using a capability | Worker-to-capability route activates; actual tool name shown |
| Tool evidence | Observation received | Result appears in the evidence stream; success/failure comes from the observation |
| Reviewing | Checking the result | Review route becomes active; completed worker cards remain |
| Repair round increases | Revising | Current tasks update; prior tool observations remain inspectable |
| Completed | Ready | Core settles; result card enters; composer returns to full size |
| Needs input / Blocked / Interrupted | Attention needed | Motion stops; actual explanation and resolution controls remain visible |
| Stopping / Stopped | Stopping / Stopped | No simulated continuation; Stop remains available until acknowledged |
| Upgrading | Runtime handoff | Explicit handoff state, never portrayed as a completed goal |
| Offline | Connection lost | Freeze activity animation and label the last known state |

## Layout

- Production header: goal, runtime state, graph/conversation control.
- Model panel: one persistent AI core, selected provider/model, current request role.
- Worker lane: real sequential assignments, current/done/queued states, read-only inspection.
- Capability panel: files, terminal, browser, desktop, app APIs, skills/tools, memory,
  and runtime. Distinguish available, disabled, used, active, and failed.
- Relationship SVG: planner assignments, model/worker connection, and observed tool
  usage. Idle availability is not a claim that a tool has run.
- Evidence stream: recent real observations, with the existing full ledger beneath.
- Result handoff: real final message and access to the full conversation.

Workers are sequential in the current backend. The diagram must not imply parallel
agents, independent LLMs, token streaming, hidden chain-of-thought, or measured model
progress. “Thinking” means a model response is pending, not disclosure of reasoning.

## Implementation architecture

`production.js` owns a presentation state machine and renders snapshots supplied by
`chat.js`. It owns no execution APIs or task scheduling. `production.css` contains
layout states and reusable motion primitives. Existing chat and recovery controls
remain the underlying interface.

Motion vocabulary: converge (composer to core), dock (core to model panel), unfold
(assignments), connect (relationship strokes), pulse (pending model request), settle
(completion), and hold (blocked/offline). Effects use transforms/opacity and bounded
durations. Core movement uses measured source/target rectangles rather than arbitrary
viewport coordinates. Animated overlays are decorative and never capture input.

Snapshot-derived graph nodes use stable IDs. Inspector text is escaped or set via
`textContent`; imported tool names and model messages are untrusted. Fast operations
may be observed only as completed evidence because the server is polled. Do not replay
old tool activity as if live. Long histories are summarized visually, with full evidence
retained in the existing ledger.

## Accessibility and responsiveness

Reduced motion presents the final layout immediately without travel, pulses or packet
animation. Status is announced only when it changes. All graph nodes are keyboard
operable. Focus is not moved when snapshots arrive. On narrow screens, lanes stack;
relationships remain readable as text and no horizontal page scrolling is introduced.
User selection of conversation view persists during a run. Rapid submit/stop/switch
must cancel obsolete motion and never reveal a stale conversation.

## Verification

Check actual scripted planning, worker/tool observations, review, completion, blocked,
stopped, repair, and offline states. Exercise graph inspection, conversation toggle,
mobile layout, reduced motion, and switching during a transition. Capture real browser
screenshots of production states as well as the unchanged landing screen.

## Verified implementation

Studio unit tests (22) and chat integration tests (10) passed on Windows. The new
production test executes a scripted model sequence with two real worker assignments,
file writing/read-back, a second worker reading the result, model review and completion.
It verifies the pending-model signal, graph relationships, node inspection, switching
away and back during execution, the conversation toggle, and a 390px mobile layout.

Separate presentation fixtures verify routing for all eight capability families,
including native serialized `RunShell`/`FetchUrl` actions and their evidence strings,
plus blocked/offline and reduced-motion behavior. Those fixtures do not execute
extra tools. Final targeted browser verification, Clippy with warnings denied,
JavaScript syntax and Rust formatting passed. Thinking, working, mobile and settled
result screenshots were captured and inspected in `workspace/studio-qa/production-*.png`.

The build is served by the local Studio preview. Reload the page and send a new
instruction to see the full entrance sequence; opening saved work starts at its
current state. The final result appears above the graph. Full evidence and resume /
uncertain-action controls remain underneath, and Conversation switches back to chat.
